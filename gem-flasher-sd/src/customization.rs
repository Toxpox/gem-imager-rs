use crate::helpers::check_cancel;
use crate::{Error, Result};
use fatfs::FileSystem;
use fscommon::{BufStream, StreamSlice};
use gem_helper::cancel::CancellationToken;
use std::io::{Read, Seek, SeekFrom, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParitionType {
    Boot,
}

impl ParitionType {
    pub(crate) fn open<T>(&self, dst: T) -> Result<FileSystem<BufStream<StreamSlice<T>>>>
    where
        T: Write + Seek + Read + std::fmt::Debug,
    {
        match self {
            Self::Boot => Self::boot_partition(dst),
        }
    }

    fn boot_partition<T>(mut dst: T) -> Result<FileSystem<BufStream<StreamSlice<T>>>>
    where
        T: Write + Seek + Read + std::fmt::Debug,
    {
        dst.rewind()?;
        let part_table = PartitionTable::detect_partition_table(&mut dst)?;
        dst.rewind()?;
        let candidates = match part_table {
            PartitionTable::Gpt => {
                let disk = gpt::GptConfig::new()
                    .writable(false)
                    .open_from_device(&mut dst)
                    .map_err(|_| crate::Error::InvalidPartitionTable)?;

                let sector = gpt::disk::DEFAULT_SECTOR_SIZE.as_u64();
                let mut entries: Vec<(u32, u64, u64)> = disk
                    .partitions()
                    .iter()
                    .filter(|(_, part)| part.last_lba > part.first_lba)
                    .map(|(index, part)| (*index, part.first_lba * sector, part.last_lba * sector))
                    .collect();
                entries.sort_unstable_by_key(|(index, _, _)| *index);
                entries
            }
            PartitionTable::Mbr => {
                let mbr = mbrman::MBRHeader::read_from(&mut dst)
                    .map_err(|_| Error::InvalidPartitionTable)?;

                let mut entries: Vec<(u32, u64, u64)> = mbr
                    .iter()
                    .filter(|(_, part)| part.sectors > 0 && part.starting_lba > 0)
                    .map(|(index, part)| {
                        let start = u64::from(part.starting_lba) * 512;
                        (index as u32, start, start + u64::from(part.sectors) * 512)
                    })
                    .collect();
                entries.sort_unstable_by_key(|(index, _, _)| *index);
                entries
            }
        };

        if candidates.is_empty() {
            return Err(Error::InvalidPartitionTable);
        }

        let mut boot = None;
        for (_, start_offset, end_offset) in candidates {
            if looks_like_fat(&mut dst, start_offset)? {
                boot = Some((start_offset, end_offset));
                break;
            }
        }

        let (start_offset, end_offset) = boot.ok_or(Error::InvalidBootPartition)?;

        dst.rewind()?;
        let slice = StreamSlice::new(dst, start_offset, end_offset)
            .map_err(|_| Error::InvalidPartitionTable)?;
        let boot_stream = BufStream::new(slice);
        FileSystem::new(boot_stream, fatfs::FsOptions::new())
            .map_err(|_| Error::InvalidBootPartition)
    }
}

fn looks_like_fat<T>(dst: &mut T, start_offset: u64) -> Result<bool>
where
    T: Read + Seek,
{
    let mut sector = [0u8; 512];

    dst.seek(SeekFrom::Start(start_offset))?;
    if dst.read_exact(&mut sector).is_err() {
        return Ok(false);
    }

    if sector[510] != 0x55 || sector[511] != 0xAA {
        return Ok(false);
    }

    let bytes_per_sector = u16::from_le_bytes([sector[11], sector[12]]);
    if !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096) {
        return Ok(false);
    }

    if sector[13] == 0 {
        return Ok(false);
    }

    let fat_marker = |window: &[u8]| window.starts_with(b"FAT");

    Ok(fat_marker(&sector[54..59]) || fat_marker(&sector[82..87]))
}

#[derive(Debug)]
enum PartitionTable {
    Gpt,
    Mbr,
}

impl PartitionTable {
    fn detect_partition_table(mut reader: impl Read) -> Result<PartitionTable> {
        let mut buf = [0u8; 1024];
        reader.read_exact(&mut buf)?;

        if &buf[512..520] == b"EFI PART" {
            return Ok(PartitionTable::Gpt);
        }

        if buf[510] == 0x55 && buf[511] == 0xAA {
            return Ok(PartitionTable::Mbr);
        }

        Err(crate::Error::InvalidPartitionTable)
    }
}

pub enum ContentType<'a> {
    Dir,
    Reader(Box<dyn Read + 'a>),
    File(Box<std::path::Path>),
    DataAppend(Box<[u8]>),
    VerifiedData(Box<[u8]>),
}

impl<'a> From<Box<[u8]>> for ContentType<'a> {
    fn from(value: Box<[u8]>) -> Self {
        Self::DataAppend(value)
    }
}

impl<'a> From<Box<std::path::Path>> for ContentType<'a> {
    fn from(value: Box<std::path::Path>) -> Self {
        Self::File(value)
    }
}

#[derive(Clone, Debug)]
pub struct Customization<I> {
    pub partition: ParitionType,
    pub content: I,
}

impl<'a, I> Customization<I>
where
    I: Iterator<Item = (Box<str>, ContentType<'a>)>,
{
    pub(crate) fn customize(
        self,
        mut dst: impl Write + Seek + Read + std::fmt::Debug,
        cancel: Option<CancellationToken>,
    ) -> Result<()> {
        let mut to_verify: Vec<(Box<str>, Box<[u8]>)> = Vec::new();

        let partition = self.partition.open(&mut dst)?;
        {
            let root = partition.root_dir();

            for (path, data) in self.content {
                let customization_err = |source| Error::CustomizationFileCreateFail {
                    source,
                    file: path.clone(),
                };
                crate::helpers::check_cancel(cancel.as_ref())?;

                match data {
                    ContentType::File(spath) => {
                        let mut f = root.create_file(&path).map_err(customization_err)?;
                        let mut source = std::fs::File::open(spath)?;
                        std::io::copy(&mut source, &mut f)?;
                    }
                    ContentType::DataAppend(items) => {
                        let mut f = root.create_file(&path).map_err(customization_err)?;
                        f.seek(SeekFrom::End(0))?;
                        f.write_all(&items)?;
                    }
                    ContentType::Dir => {
                        root.create_dir(&path)?;
                    }
                    ContentType::Reader(mut reader) => {
                        let mut dst = root.create_file(&path).map_err(customization_err)?;
                        dst.truncate()?;
                        std::io::copy(&mut reader, &mut dst)?;
                    }
                    ContentType::VerifiedData(items) => {
                        let mut f = root.create_file(&path).map_err(customization_err)?;
                        f.truncate()?;
                        f.write_all(&items)?;
                        f.flush()?;
                        to_verify.push((path, items));
                    }
                }
            }
        }

        partition.unmount()?;

        if !to_verify.is_empty() {
            check_cancel(cancel.as_ref())?;
            self.partition.verify(&mut dst, &to_verify)?;
        }

        Ok(())
    }
}

impl ParitionType {
    pub(crate) fn verify<T>(self, dst: T, expected: &[(Box<str>, Box<[u8]>)]) -> Result<()>
    where
        T: Write + Seek + Read + std::fmt::Debug,
    {
        let partition = self.open(dst)?;
        {
            let root = partition.root_dir();

            for (path, want) in expected {
                let mut got = Vec::with_capacity(want.len());
                root.open_file(path)
                    .map_err(|_| Error::CustomizationReadBackMismatch { file: path.clone() })?
                    .read_to_end(&mut got)?;

                if got.as_slice() != want.as_ref() {
                    return Err(Error::CustomizationReadBackMismatch { file: path.clone() });
                }
            }
        }

        partition.unmount()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpt_disk_without_partition_two() -> std::io::Cursor<Vec<u8>> {
        const DISK_SIZE: usize = 16 * 1024 * 1024;

        let mut disk = std::io::Cursor::new(vec![0u8; DISK_SIZE]);
        let mut gpt = gpt::GptConfig::new()
            .writable(true)
            .logical_block_size(gpt::disk::LogicalBlockSize::Lb512)
            .create_from_device(&mut disk, None)
            .unwrap();

        gpt.add_partition(
            "only-partition",
            4 * 1024 * 1024,
            gpt::partition_types::BASIC,
            0,
            None,
        )
        .unwrap();
        gpt.write().unwrap();

        disk.set_position(0);
        disk
    }

    fn gpt_disk_with_fat_boot_first() -> std::io::Cursor<Vec<u8>> {
        const DISK_SIZE: usize = 32 * 1024 * 1024;
        const BOOT_SIZE: u64 = 8 * 1024 * 1024;

        let mut disk = std::io::Cursor::new(vec![0u8; DISK_SIZE]);
        let mut gpt = gpt::GptConfig::new()
            .writable(true)
            .logical_block_size(gpt::disk::LogicalBlockSize::Lb512)
            .create_from_device(&mut disk, None)
            .unwrap();

        let id = gpt
            .add_partition("boot", BOOT_SIZE, gpt::partition_types::EFI, 0, None)
            .unwrap();
        gpt.add_partition(
            "root",
            8 * 1024 * 1024,
            gpt::partition_types::LINUX_FS,
            0,
            None,
        )
        .unwrap();

        let start = gpt.partitions().get(&id).unwrap().first_lba * 512;
        let end = gpt.partitions().get(&id).unwrap().last_lba * 512;
        gpt.write().unwrap();

        let slice = StreamSlice::new(&mut disk, start, end).unwrap();
        fatfs::format_volume(
            BufStream::new(slice),
            fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat32),
        )
        .unwrap();

        disk.set_position(0);
        disk
    }

    #[test]
    fn the_boot_partition_is_found_even_when_it_is_not_the_second_entry() {
        let disk = gpt_disk_with_fat_boot_first();

        ParitionType::Boot
            .open(disk)
            .expect("a FAT boot partition in slot 1 must still be customizable");
    }

    #[test]
    fn a_gpt_image_without_the_boot_partition_is_rejected_instead_of_panicking() {
        let disk = gpt_disk_without_partition_two();

        match ParitionType::Boot.open(disk) {
            Ok(_) => panic!("a GPT image without a FAT boot partition must not be customized"),
            Err(Error::InvalidBootPartition) => {}
            Err(other) => panic!("expected InvalidBootPartition, got {other:?}"),
        }
    }
}
