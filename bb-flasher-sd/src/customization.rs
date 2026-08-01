use crate::helpers::check_cancel;
use crate::{Error, Result};
use bb_helper::cancel::CancellationToken;
use fatfs::FileSystem;
use fscommon::{BufStream, StreamSlice};
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
        // Partition detection reads from wherever the stream happens to be, so start from the top.
        // Without this, opening the partition a second time — which the read-back pass does —
        // reads whatever follows the previous access and reports a corrupt partition table.
        dst.rewind()?;
        let part_table = PartitionTable::detect_partition_table(&mut dst)?;
        dst.rewind()?;
        let (start_offset, end_offset) = match part_table {
            PartitionTable::Gpt => {
                let disk = gpt::GptConfig::new()
                    .writable(false)
                    .open_from_device(&mut dst)
                    .map_err(|_| crate::Error::InvalidPartitionTable)?;

                let partition_2 = disk.partitions().get(&2).unwrap();

                let start_offset: u64 =
                    partition_2.first_lba * gpt::disk::DEFAULT_SECTOR_SIZE.as_u64();
                let end_offset: u64 =
                    partition_2.last_lba * gpt::disk::DEFAULT_SECTOR_SIZE.as_u64();

                (start_offset, end_offset)
            }
            PartitionTable::Mbr => {
                let mbr = mbrman::MBRHeader::read_from(&mut dst)
                    .map_err(|_| Error::InvalidPartitionTable)?;

                let boot_part = mbr.get(1).ok_or(Error::InvalidPartitionTable)?;
                let start_offset: u64 = (boot_part.starting_lba * 512).into();
                let end_offset: u64 = start_offset + u64::from(boot_part.sectors) * 512;

                (start_offset, end_offset)
            }
        };

        let slice = StreamSlice::new(dst, start_offset, end_offset)
            .map_err(|_| Error::InvalidPartitionTable)?;
        let boot_stream = BufStream::new(slice);
        FileSystem::new(boot_stream, fatfs::FsOptions::new())
            .map_err(|_| Error::InvalidBootPartition)
    }
}

#[derive(Debug)]
enum PartitionTable {
    Gpt,
    Mbr,
}

impl PartitionTable {
    fn detect_partition_table(mut reader: impl Read) -> Result<PartitionTable> {
        // Read first 1024 bytes (enough for MBR + GPT header)
        let mut buf = [0u8; 1024];
        reader.read_exact(&mut buf)?;

        // Check GPT signature at LBA1 (offset 512)
        if &buf[512..520] == b"EFI PART" {
            return Ok(PartitionTable::Gpt);
        }

        // Check MBR boot signature
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
    /// Replace the file with exactly these bytes, then read them back off the device and compare.
    ///
    /// This is the mode for files whose absence or corruption stays invisible until the board is
    /// booted — a first-boot configuration that silently did not land looks like a successful flash
    /// right up to the moment the user cannot log in.
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
    /// Write the customization files, then read back the ones that asked to be verified.
    ///
    /// The read-back re-opens the FAT filesystem from scratch rather than reusing the handle that
    /// did the writing, so it goes through the same path the board will: a file that only exists in
    /// a cache the writer still holds is not a file the board can read.
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
    /// Re-open the partition and confirm each file holds exactly the bytes that were written.
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

                // A plain inequality, not a diff: the buffer can hold a password hash, so nothing
                // about its contents may reach an error message or a log line.
                if got.as_slice() != want.as_ref() {
                    return Err(Error::CustomizationReadBackMismatch { file: path.clone() });
                }
            }
        }

        partition.unmount()?;

        Ok(())
    }
}
