use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::mem::{offset_of, size_of};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::time::{Duration, Instant};

use bb_helper::cancel::CancellationToken;
use windows::Win32::{
    Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_FILES, HANDLE},
    Storage::FileSystem::{
        FILE_SHARE_READ, FILE_SHARE_WRITE, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose,
        IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
    },
    System::Diagnostics::Debug::{
        GetErrorMode, SEM_FAILCRITICALERRORS, SetErrorMode, THREAD_ERROR_MODE,
    },
    System::IO::DeviceIoControl,
    System::Ioctl::{
        DISK_EXTENT, FSCTL_ALLOW_EXTENDED_DASD_IO, FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME,
        FSCTL_UNLOCK_VOLUME, IOCTL_STORAGE_EJECT_MEDIA, VOLUME_DISK_EXTENTS,
    },
};

use crate::{Error, Result};

#[derive(Debug)]
pub(crate) struct WinDrive {
    drive: File,
}

#[derive(Debug)]
struct LockedVolume {
    path: String,
    file: File,
}

const FILE_FLAG_WRITE_THROUGH: u32 = 0x80000000;
const FILE_FLAG_NO_BUFFERING: u32 = 0x20000000;
const CREATE_NO_WINDOW: u32 = 0x08000000;
const VOLUME_NAME_BUFFER_LEN: usize = 1024;
const VOLUME_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const VOLUME_LOCK_RETRY_DELAY: Duration = Duration::from_millis(200);

impl WinDrive {
    pub(crate) fn open(path: &Path) -> anyhow::Result<Self> {
        // Raw removable-media probes must return errors to the application instead of opening a
        // system modal "format this disk"/"insert a disk" dialog. Preserve every existing mode
        // bit and add only the Microsoft-recommended critical-error suppression flag.
        unsafe {
            SetErrorMode(THREAD_ERROR_MODE(GetErrorMode() | SEM_FAILCRITICALERRORS.0));
        }

        let disk_number = physical_drive_number(path)?;
        tracing::info!("Locking existing volumes on physical disk {disk_number}");
        let existing_volumes = lock_volumes_with_retry(disk_number, None)?;

        tracing::info!("Trying to clean {:?}", path);
        diskpart_clean(path)?;

        tracing::info!("Trying to open {:?}", path);
        let drive = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_WRITE_THROUGH | FILE_FLAG_NO_BUFFERING)
            .open(path)?;
        // Keep the old-layout locks until the physical disk handle is acquired. The replacement
        // layout stays hidden in `SdCardWrapper` until all verification and customization work is
        // complete, so no post-write volume enumeration is needed.
        drop(existing_volumes);

        Ok(Self { drive })
    }
}

impl Drop for LockedVolume {
    fn drop(&mut self) {
        tracing::debug!("Unlocking Windows volume {}", self.path);
        let _ = unsafe {
            DeviceIoControl(
                HANDLE(self.file.as_raw_handle()),
                FSCTL_UNLOCK_VOLUME,
                None,
                0,
                None,
                0,
                None,
                None,
            )
        };
    }
}

fn open_and_lock_volume(path: &str) -> io::Result<LockedVolume> {
    let volume = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
        .open(path)?;

    let result = unsafe {
        DeviceIoControl(
            HANDLE(volume.as_raw_handle()),
            FSCTL_ALLOW_EXTENDED_DASD_IO,
            None,
            0,
            None,
            0,
            None,
            None,
        )
    };
    result.map_err(|_| io::Error::last_os_error())?;

    let result = unsafe {
        DeviceIoControl(
            HANDLE(volume.as_raw_handle()),
            FSCTL_LOCK_VOLUME,
            None,
            0,
            None,
            0,
            None,
            None,
        )
    };
    result.map_err(|_| io::Error::last_os_error())?;

    let result = unsafe {
        DeviceIoControl(
            HANDLE(volume.as_raw_handle()),
            FSCTL_DISMOUNT_VOLUME,
            None,
            0,
            None,
            0,
            None,
            None,
        )
    };
    result.map_err(|_| io::Error::last_os_error())?;

    Ok(LockedVolume {
        path: path.to_owned(),
        file: volume,
    })
}

fn physical_drive_number(drive: &Path) -> anyhow::Result<u32> {
    drive
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Drive path is not valid UTF-8"))?
        .strip_prefix("\\\\.\\PhysicalDrive")
        .ok_or_else(|| anyhow::anyhow!("Drive path is not a physical disk"))?
        .parse()
        .map_err(Into::into)
}

#[derive(Debug)]
struct VolumeSearch(HANDLE);

impl Drop for VolumeSearch {
    fn drop(&mut self) {
        let _ = unsafe { FindVolumeClose(self.0) };
    }
}

fn volume_name_from_buffer(buffer: &[u16]) -> io::Result<String> {
    let end = buffer
        .iter()
        .position(|c| *c == 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unterminated volume name"))?;
    String::from_utf16(&buffer[..end]).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn enumerate_volume_paths() -> io::Result<Vec<String>> {
    let mut buffer = [0u16; VOLUME_NAME_BUFFER_LEN];
    let search =
        unsafe { FindFirstVolumeW(&mut buffer) }.map_err(|_| io::Error::last_os_error())?;
    let search = VolumeSearch(search);
    let mut volumes = Vec::new();

    loop {
        let volume = volume_name_from_buffer(&buffer)?;
        let device_path = volume.strip_suffix('\\').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid volume GUID path")
        })?;
        volumes.push(device_path.to_owned());
        buffer.fill(0);

        match unsafe { FindNextVolumeW(search.0, &mut buffer) } {
            Ok(()) => {}
            Err(_) => {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_NO_MORE_FILES.0 as i32) {
                    break;
                }
                return Err(error);
            }
        }
    }

    Ok(volumes)
}

fn extent_buffer_len(extent_count: usize) -> usize {
    offset_of!(VOLUME_DISK_EXTENTS, Extents) + extent_count * size_of::<DISK_EXTENT>()
}

fn parse_disk_numbers(buffer: &[usize], returned: usize) -> io::Result<Vec<u32>> {
    if returned < offset_of!(VOLUME_DISK_EXTENTS, Extents) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "volume extent response was shorter than its header",
        ));
    }

    let header = buffer.as_ptr().cast::<VOLUME_DISK_EXTENTS>();
    let count = usize::try_from(unsafe { (*header).NumberOfDiskExtents }).unwrap();
    let needed = extent_buffer_len(count);
    if needed > returned || needed > std::mem::size_of_val(buffer) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "volume extent response did not contain every reported extent",
        ));
    }

    let extents = unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!((*header).Extents).cast::<DISK_EXTENT>(),
            count,
        )
    };
    Ok(extents.iter().map(|extent| extent.DiskNumber).collect())
}

fn volume_disk_numbers(path: &str) -> io::Result<Vec<u32>> {
    let volume = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
        .open(path)?;

    let word_size = size_of::<usize>();
    let initial_len = size_of::<VOLUME_DISK_EXTENTS>().div_ceil(word_size);
    let mut buffer = vec![0usize; initial_len];

    loop {
        let mut returned = 0u32;
        let result = unsafe {
            DeviceIoControl(
                HANDLE(volume.as_raw_handle()),
                IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
                None,
                0,
                Some(buffer.as_mut_ptr().cast()),
                u32::try_from(std::mem::size_of_val(&buffer)).unwrap(),
                Some(&mut returned),
                None,
            )
        };

        match result {
            Ok(()) => return parse_disk_numbers(&buffer, returned as usize),
            Err(_) => {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_MORE_DATA.0 as i32) {
                    return Err(error);
                }

                let header = buffer.as_ptr().cast::<VOLUME_DISK_EXTENTS>();
                let count = usize::try_from(unsafe { (*header).NumberOfDiskExtents }).unwrap();
                let needed = extent_buffer_len(count);
                if needed <= std::mem::size_of_val(&buffer) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Windows requested a larger extent buffer without reporting its size",
                    ));
                }
                buffer.resize(needed.div_ceil(word_size), 0);
            }
        }
    }
}

fn target_volume_paths(disk_number: u32) -> Result<Vec<String>> {
    let volumes =
        enumerate_volume_paths().map_err(|source| Error::WindowsVolumeEnumeration { source })?;
    let mut targets = Vec::new();

    for volume in volumes {
        let disk_numbers = match volume_disk_numbers(&volume) {
            Ok(numbers) => numbers,
            Err(error) => {
                tracing::debug!("Skipping volume {volume}: cannot query disk extents: {error}");
                continue;
            }
        };

        if !disk_numbers.contains(&disk_number) {
            continue;
        }
        if disk_numbers.iter().any(|number| *number != disk_number) {
            return Err(Error::WindowsSpannedVolume {
                volume: volume.into(),
                disk_number,
            });
        }

        targets.push(volume);
    }

    Ok(targets)
}

fn lock_volumes_once(disk_number: u32) -> Result<Vec<LockedVolume>> {
    let targets = target_volume_paths(disk_number)?;
    let mut locked = Vec::with_capacity(targets.len());

    for volume in targets {
        tracing::info!("Locking Windows volume {volume} on physical disk {disk_number}");
        let handle = open_and_lock_volume(&volume).map_err(|source| Error::WindowsVolumeLock {
            source,
            volume: volume.clone().into(),
        })?;
        locked.push(handle);
    }

    Ok(locked)
}

fn lock_volumes_with_retry(
    disk_number: u32,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<LockedVolume>> {
    let deadline = Instant::now() + VOLUME_LOCK_TIMEOUT;
    let mut attempt = 0u32;

    loop {
        crate::helpers::check_cancel(cancel)?;
        attempt += 1;

        let error = match lock_volumes_once(disk_number) {
            Ok(volumes) => {
                tracing::info!(
                    "Locked {} Windows volume(s) on physical disk {disk_number} after {attempt} attempt(s)",
                    volumes.len()
                );
                return Ok(volumes);
            }
            Err(error) => {
                tracing::debug!(
                    "Volume lock attempt {attempt} for physical disk {disk_number} failed: {error}"
                );
                error
            }
        };

        let now = Instant::now();
        if now >= deadline {
            return Err(error);
        }
        std::thread::sleep(VOLUME_LOCK_RETRY_DELAY.min(deadline - now));
    }
}

fn diskpart_clean(path: &Path) -> Result<()> {
    let disk_num = path
        .to_str()
        .unwrap()
        .strip_prefix("\\\\.\\PhysicalDrive")
        .ok_or(io::Error::new(io::ErrorKind::NotFound, "Drive not found"))?;

    let resp = std::process::Command::new("powershell")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Clear-Disk",
            "-Number",
            disk_num,
            "-RemoveData",
            "-Confirm:$false",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    tracing::info!("Disk Clear Response: {:#?}", resp);

    if resp.status.success() {
        Ok(())
    } else {
        Err(Error::WindowsCleanError(resp))
    }
}

impl Read for WinDrive {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.drive.read(buf)
    }
}

impl Write for WinDrive {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.drive.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drive.flush()
    }
}

impl Seek for WinDrive {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.drive.seek(pos)
    }
}

/// TODO: Implement real eject
impl crate::helpers::Commit for WinDrive {
    fn commit(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.drive)?;
        self.drive.sync_all()
    }
}

impl crate::helpers::Eject for WinDrive {
    fn eject(mut self) -> io::Result<()> {
        crate::helpers::Commit::commit(&mut self)?;
        unsafe {
            DeviceIoControl(
                HANDLE(self.drive.as_raw_handle()),
                IOCTL_STORAGE_EJECT_MEDIA,
                None,
                0,
                None,
                0,
                None,
                None,
            )
        }
        .map_err(|_| io::Error::last_os_error())
    }
}

pub(crate) fn format(dst: &Path) -> Result<()> {
    let disk_size = crate::helpers::destination_size(dst)?;
    let drive = open(dst)?;
    crate::helpers::format_device(drive, disk_size)
}

pub(crate) fn open(dst: &Path) -> Result<WinDrive> {
    WinDrive::open(dst).map_err(|e| Error::FailedToOpenDestination { source: e })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent_response(disk_numbers: &[u32]) -> (Vec<usize>, usize) {
        let bytes = extent_buffer_len(disk_numbers.len());
        let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let header = buffer.as_mut_ptr().cast::<VOLUME_DISK_EXTENTS>();

        unsafe {
            (*header).NumberOfDiskExtents = u32::try_from(disk_numbers.len()).unwrap();
            let extents = std::ptr::addr_of_mut!((*header).Extents).cast::<DISK_EXTENT>();
            for (index, disk_number) in disk_numbers.iter().enumerate() {
                (*extents.add(index)).DiskNumber = *disk_number;
            }
        }

        (buffer, bytes)
    }

    #[test]
    fn physical_drive_number_accepts_the_canonical_windows_path() {
        assert_eq!(
            physical_drive_number(Path::new(r"\\.\PhysicalDrive17")).unwrap(),
            17
        );
    }

    #[test]
    fn physical_drive_number_rejects_non_disk_paths() {
        assert!(physical_drive_number(Path::new(r"\\.\E:")).is_err());
    }

    #[test]
    fn extent_parser_reads_every_reported_disk_number() {
        let (buffer, returned) = extent_response(&[3, 9, 12]);
        assert_eq!(
            parse_disk_numbers(&buffer, returned).unwrap(),
            vec![3, 9, 12]
        );
    }

    #[test]
    fn extent_parser_rejects_a_truncated_response() {
        let (buffer, returned) = extent_response(&[3, 9]);
        assert!(parse_disk_numbers(&buffer, returned - 1).is_err());
    }

    #[test]
    fn volume_name_parser_stops_at_the_first_nul() {
        let input: Vec<u16> = r"\\?\Volume{test}\"
            .encode_utf16()
            .chain([0, b'X' as u16])
            .collect();
        assert_eq!(
            volume_name_from_buffer(&input).unwrap(),
            r"\\?\Volume{test}\"
        );
    }
}
