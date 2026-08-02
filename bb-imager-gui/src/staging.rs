//! Staging area for the DFU/eMMC write path.
//!
//! The SD path streams the extracted image straight at the card. DFU cannot do that: the boot
//! chain has to be transferred first, the raw eMMC stage needs a known byte count up front, and the
//! T3 first-boot file is written into the image's FAT partition rather than onto a device. So the
//! DFU path materialises one **staging image** — the extracted, customized, read-back-verified
//! bytes — and then streams that file to the board.
//!
//! Two failure modes are handled here rather than discovered halfway through:
//!
//! * **Disk space.** A 4 GiB staging image on a full disk fails after the whole download. The
//!   required size is known before the first byte is fetched, so it is checked first.
//! * **Leftovers.** A staging image is a full copy of a bootable OS *including the user's Wi-Fi
//!   PSK and password hash* (`instruction.md` §10.3). It is deleted when the write ends, on
//!   cancellation, and — for the case no `Drop` can cover, a crash or a kill — swept at start-up.

use std::{
    io,
    path::{Path, PathBuf},
};

/// Extra room demanded beyond the image itself.
///
/// Filling a disk to the last byte breaks the rest of the system, and the SD writer needs room for
/// its own bookkeeping while it applies customization.
const HEADROOM: u64 = 256 * 1024 * 1024;

/// Prefix every staging file shares, so the start-up sweep can recognise its own leftovers and
/// nothing else.
const PREFIX: &str = "t3-staging-";

#[derive(Debug, thiserror::Error)]
pub(crate) enum StagingError {
    #[error("no application cache directory is available for the DFU staging image")]
    NoCacheDir,
    #[error("failed to prepare the DFU staging directory: {source}")]
    Io {
        #[source]
        source: io::Error,
    },
    /// Worded so the operator learns the number that matters, and matched by
    /// `message::localized_flash_error` on the word "staging".
    #[error(
        "not enough free space for the DFU staging image: {required} bytes required, \
         {available} bytes available"
    )]
    InsufficientSpace { required: u64, available: u64 },
}

/// Directory staging images live in.
pub(crate) fn staging_dir() -> Result<PathBuf, StagingError> {
    let dirs = crate::helpers::project_dirs().ok_or(StagingError::NoCacheDir)?;
    Ok(dirs.cache_dir().join("dfu-staging"))
}

/// A staging image that deletes itself.
#[derive(Debug)]
pub(crate) struct StagingImage {
    path: PathBuf,
}

impl StagingImage {
    /// Reserve a staging path for an image of `image_size` bytes.
    ///
    /// The space check happens here — before the caller starts downloading — because the whole
    /// point is not to spend an hour on a write that cannot land.
    pub(crate) fn create(image_size: u64) -> Result<Self, StagingError> {
        let dir = staging_dir()?;
        std::fs::create_dir_all(&dir).map_err(|source| StagingError::Io { source })?;

        let required = image_size.saturating_add(HEADROOM);
        let available = available_space(&dir).map_err(|source| StagingError::Io { source })?;
        if available < required {
            return Err(StagingError::InsufficientSpace {
                required,
                available,
            });
        }

        // The name only has to be unique among live processes; the sweep cleans up anything an
        // earlier one left behind.
        let path = dir.join(format!("{PREFIX}{}.img", std::process::id()));
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StagingImage {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => tracing::info!("Removed the DFU staging image"),
            // The common case: the write failed before the file was created.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            // A staging image carries the user's secrets, so a failure to remove it is worth a
            // warning even though it cannot fail the flash that already finished.
            Err(e) => tracing::warn!("Failed to remove the DFU staging image: {e}"),
        }
    }
}

/// Delete staging images left behind by a previous run.
///
/// Best-effort and never fatal: it runs at start-up, where the only alternative to logging is
/// refusing to start over a stale temporary file.
pub(crate) fn cleanup_stale() {
    let Ok(dir) = staging_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(PREFIX) {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => tracing::info!("Removed a stale DFU staging image from a previous run"),
            Err(e) => tracing::warn!("Failed to remove a stale DFU staging image: {e}"),
        }
    }
}

/// Bytes still writable in the filesystem holding `dir`.
#[cfg(windows)]
fn available_space(dir: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt as _;

    // `GetDiskFreeSpaceExW` reports the quota-aware figure for the calling user, which is the
    // number that decides whether *this* process can write the file.
    let mut wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    wide.push(0);

    let mut free_for_caller: u64 = 0;
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call, and the out-pointer
    // refers to a live local. The two trailing out-parameters are optional and passed as null.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_for_caller,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };

    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(free_for_caller)
}

#[cfg(unix)]
fn available_space(dir: &Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt as _;

    let mut path = dir.as_os_str().as_bytes().to_vec();
    path.push(0);

    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is NUL-terminated and outlives the call; `stat` is a live, zeroed value of
    // exactly the type the call fills in.
    let rc = unsafe { libc::statvfs(path.as_ptr().cast(), &mut stat) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }

    // `f_bavail` rather than `f_bfree`: the reserved-for-root blocks are not writable by the user
    // running the GUI, and counting them would turn a pre-check into a false pass.
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_space_reports_a_real_figure_for_the_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let free = available_space(dir.path()).unwrap();
        // Any machine that can run this test has more than a megabyte free; the assertion is that
        // the platform call was made and parsed, not that the disk is large.
        assert!(free > 1024 * 1024, "implausible free space: {free}");
    }

    #[test]
    fn a_request_larger_than_the_disk_is_refused_before_any_download() {
        let err = StagingImage::create(u64::MAX - HEADROOM).unwrap_err();
        assert!(
            matches!(err, StagingError::InsufficientSpace { required, .. } if required == u64::MAX),
            "expected an insufficient-space refusal, got {err}"
        );
        // The message has to name the shortage, because that is what the localized reducer keys on.
        assert!(err.to_string().contains("staging"));
    }

    #[test]
    fn the_staging_image_is_removed_when_it_goes_out_of_scope() {
        let staging = StagingImage::create(0).unwrap();
        let path = staging.path().to_path_buf();
        std::fs::write(&path, b"secret bytes").unwrap();
        assert!(path.exists());

        drop(staging);
        assert!(!path.exists(), "a staging image outlived its guard");
    }

    /// The case no `Drop` can cover: the process was killed while a staging image existed.
    #[test]
    fn stale_images_from_a_previous_run_are_swept() {
        let dir = staging_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let stale = dir.join(format!("{PREFIX}stale-test.img"));
        let unrelated = dir.join("keep-me.txt");
        std::fs::write(&stale, b"leftover").unwrap();
        std::fs::write(&unrelated, b"not ours").unwrap();

        cleanup_stale();

        assert!(!stale.exists());
        assert!(
            unrelated.exists(),
            "the sweep must only remove files it created"
        );
        let _ = std::fs::remove_file(unrelated);
    }
}
