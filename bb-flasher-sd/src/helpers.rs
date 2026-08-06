use std::io::{self, Write};

use bb_helper::cancel::CancellationToken;
use std::sync::mpsc;

/// Best-effort progress report. A full or closed channel must never fail a flash.
pub(crate) fn chan_send<T>(chan: Option<&mpsc::SyncSender<T>>, msg: T) {
    if let Some(c) = chan {
        let _ = c.try_send(msg);
    }
}

/// Fraction of `img_size` reached, clamped to `1.0`.
///
/// The last chunk read from the image is padded up to the 512-byte alignment the device wants, so
/// `pos` can legitimately overshoot `img_size` by up to 511 bytes. Reporting `1.02` would make the
/// GUI's ETA extrapolation produce a negative duration.
pub(crate) fn progress(pos: u64, img_size: u64) -> f32 {
    if img_size == 0 {
        return 1.0;
    }

    (pos as f32 / img_size as f32).clamp(0.0, 1.0)
}

pub(crate) fn check_cancel(tkn: Option<&CancellationToken>) -> crate::Result<()> {
    if let Some(t) = tkn
        && t.is_cancelled()
    {
        Err(crate::Error::Aborted)
    } else {
        Ok(())
    }
}

/// Make every byte written so far durable on the physical device.
///
/// Split out of [`Eject`] because the read-back verification has to run *between* the write and the
/// eject: reading back through our own dirty buffers would only prove the buffers agree with
/// themselves. A failure here is a flashing failure, never a warning — an unwritten tail is exactly
/// the case where the user pulls the card and gets a board that will not boot.
pub(crate) trait Commit {
    fn commit(&mut self) -> io::Result<()>;
}

/// Publish a partition layout that was kept hidden during raw verification and customization.
pub(crate) trait PublishLayout {
    fn publish_layout(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

impl Commit for std::fs::File {
    fn commit(&mut self) -> io::Result<()> {
        self.flush()?;
        self.sync_all()
    }
}

impl PublishLayout for std::fs::File {}

impl<T: Commit + ?Sized> Commit for &mut T {
    fn commit(&mut self) -> io::Result<()> {
        (**self).commit()
    }
}

impl<T: PublishLayout + ?Sized> PublishLayout for &mut T {
    fn publish_layout(&mut self) -> crate::Result<()> {
        (**self).publish_layout()
    }
}

pub(crate) trait Eject: Commit {
    fn eject(self) -> io::Result<()>;
}

impl Eject for std::fs::File {
    fn eject(mut self) -> io::Result<()> {
        self.commit()
    }
}

/// Resolve the destination capacity and reject a disk that the platform marks as a system disk.
pub(crate) fn destination_size(path: &std::path::Path) -> crate::Result<u64> {
    let device = crate::devices(false)
        .into_iter()
        .find(|device| device.path == path)
        .ok_or_else(|| crate::Error::FailedToFormat {
            source: io::Error::new(
                io::ErrorKind::NotFound,
                format!("could not determine the size of {}", path.display()),
            ),
        })?;

    if device.is_system {
        return Err(crate::Error::SystemDisk {
            name: device.name.into(),
        });
    }

    if device.size == 0 {
        return Err(crate::Error::FailedToFormat {
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                format!("could not determine the size of {}", path.display()),
            ),
        });
    }

    Ok(device.size)
}

/// Build a single FAT32 partition while its MBR remains hidden, then publish and verify it.
pub(crate) fn format_device<D>(drive: D, disk_size: u64) -> crate::Result<()>
where
    D: io::Read + io::Write + io::Seek + Eject + std::fmt::Debug,
{
    use fscommon::StreamSlice;
    use mbrman::{BOOT_INACTIVE, CHS, MBR, MBRHeader, MBRPartitionEntry};
    use std::time::{SystemTime, UNIX_EPOCH};

    const SECTOR_SIZE: u32 = 512;
    const FIRST_LBA: u32 = 2048;

    let failed = |source| crate::Error::FailedToFormat { source };
    let mut sd = SdCardWrapper::new(drive);
    sd.hide_layout(Some(disk_size)).map_err(failed)?;

    let mut device = DeviceWrapper::new(sd).map_err(failed)?;
    let total_sectors = disk_size / u64::from(SECTOR_SIZE);
    if total_sectors <= u64::from(FIRST_LBA) {
        return Err(failed(io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination is too small for an aligned FAT32 partition",
        )));
    }

    let partition_sectors = u32::try_from(total_sectors - u64::from(FIRST_LBA)).map_err(|_| {
        failed(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MBR FAT32 formatting supports destinations up to 2 TiB",
        ))
    })?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let disk_signature = (stamp as u32 ^ (stamp >> 32) as u32).to_le_bytes();
    let mut mbr = MBR {
        sector_size: SECTOR_SIZE,
        header: MBRHeader::new(disk_signature),
        logical_partitions: Vec::new(),
        align: FIRST_LBA,
        cylinders: 0,
        heads: 0,
        sectors: 0,
        disk_size: u32::try_from(total_sectors).unwrap_or(u32::MAX),
    };
    mbr[1] = MBRPartitionEntry {
        boot: BOOT_INACTIVE,
        first_chs: CHS::empty(),
        sys: 0x0c,
        last_chs: CHS::empty(),
        starting_lba: FIRST_LBA,
        sectors: partition_sectors,
    };
    mbr.write_into(&mut device)
        .map_err(|error| failed(io::Error::other(error)))?;

    let start = u64::from(FIRST_LBA) * u64::from(SECTOR_SIZE);
    let end = start + u64::from(partition_sectors) * u64::from(SECTOR_SIZE);
    let mut partition = StreamSlice::new(&mut device, start, end).map_err(failed)?;
    fatfs::format_volume(
        &mut partition,
        fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat32),
    )
    .map_err(failed)?;
    partition.flush().map_err(failed)?;
    drop(partition);

    let mut sd = device.into_inner();
    sd.commit()
        .map_err(|source| crate::Error::SyncFailed { source })?;
    sd.publish_layout()?;
    sd.eject().map_err(failed)
}

const BLOCK_SIZE: usize = 4096;

#[derive(Debug)]
/// Wrapper to perform aligned read/write operations.
pub(crate) struct DeviceWrapper<F> {
    f: F,
    offset: u64,
    buf: Box<DirectIoBuffer<BLOCK_SIZE>>,
    cache_offset: u64,
}

impl<F> DeviceWrapper<F> {
    /// Start offset of current block
    const fn block_offset(&self) -> u64 {
        self.offset - self.cache_buf_offset() as u64
    }

    /// Offset inside cache to start reading/writing
    const fn cache_buf_offset(&self) -> usize {
        (self.offset % BLOCK_SIZE as u64) as usize
    }

    /// Number of bytes from `Self::cache_buf_offset` that can be used
    const fn cache_buf_hit_len(&self) -> usize {
        self.buf.len() - self.cache_buf_offset()
    }

    pub(crate) fn into_inner(self) -> F {
        self.f
    }
}

impl<F> DeviceWrapper<F>
where
    F: io::Seek,
{
    pub(crate) fn new(mut f: F) -> io::Result<Self> {
        f.rewind()?;
        Ok(Self {
            f,
            offset: 0,
            // Hack to make reading from 0 working
            cache_offset: 1,
            buf: Box::new(DirectIoBuffer::new()),
        })
    }
}

impl<F> DeviceWrapper<F>
where
    F: io::Read + io::Seek,
{
    fn fill_cache(&mut self) -> io::Result<()> {
        if self.cache_offset != self.block_offset() {
            self.cache_offset = self.block_offset();
            self.f.seek(io::SeekFrom::Start(self.cache_offset))?;
            self.f.read_exact(self.buf.as_mut_slice())
        } else {
            Ok(())
        }
    }
}

impl<F> io::Read for DeviceWrapper<F>
where
    F: io::Read + io::Seek,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.fill_cache()?;
        let count = std::cmp::min(buf.len(), self.cache_buf_hit_len());

        buf[..count].copy_from_slice(
            &self.buf.as_slice()[self.cache_buf_offset()..(self.cache_buf_offset() + count)],
        );

        self.offset += count as u64;

        Ok(count)
    }
}

impl<F> io::Write for DeviceWrapper<F>
where
    F: io::Write + io::Read + io::Seek,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.fill_cache()?;
        let count = std::cmp::min(buf.len(), self.cache_buf_hit_len());
        let start = self.cache_buf_offset();

        self.buf.as_mut_slice()[start..(start + count)].copy_from_slice(&buf[..count]);

        self.f.seek(io::SeekFrom::Start(self.cache_offset))?;
        self.f.write(self.buf.as_slice())?;

        self.offset += count as u64;

        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.f.flush()
    }
}

impl<F> io::Seek for DeviceWrapper<F>
where
    F: io::Seek,
{
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match pos {
            io::SeekFrom::Start(i) => self.offset = i,
            io::SeekFrom::Current(i) => self.offset = self.offset.checked_add_signed(i).unwrap(),
            io::SeekFrom::End(_) => self.offset = self.f.seek(pos)?,
        }

        Ok(self.offset)
    }
}

impl<F> PublishLayout for DeviceWrapper<F>
where
    F: PublishLayout,
{
    fn publish_layout(&mut self) -> crate::Result<()> {
        self.f.publish_layout()
    }
}

#[repr(align(4096))]
#[derive(Debug)]
pub(crate) struct DirectIoBuffer<const N: usize>([u8; N]);

impl<const N: usize> DirectIoBuffer<N> {
    pub(crate) const fn new() -> Self {
        Self([0u8; N])
    }

    pub(crate) const fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub(crate) const fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.0
    }

    const fn len(&self) -> usize {
        self.0.len()
    }
}

/// A wrapper to support writing the first block at the end. This is required on Windows to make
/// things work reliably.
///
/// Once [`Self::finish`] has run, the deferred block is on the device and the cache is retired:
/// every later read and write goes straight to `inner`. That is what makes the post-write read-back
/// meaningful — otherwise the first block would be compared against the very buffer it came from.
#[derive(Debug)]
pub(crate) struct SdCardWrapper<W> {
    inner: W,
    buf: Box<DirectIoBuffer<BLOCK_SIZE>>,
    pos: u64,
    finished: bool,
}

impl<W> SdCardWrapper<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self {
            inner,
            buf: Box::new(DirectIoBuffer::new()),
            pos: 0,
            finished: false,
        }
    }

    /// Whether the deferred first block is still only in memory.
    const fn is_cached(&self, pos: usize) -> bool {
        !self.finished && pos < self.buf.len()
    }
}

impl<W> SdCardWrapper<W>
where
    W: io::Write + io::Seek,
{
    fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }

        self.inner.seek(io::SeekFrom::Start(0))?;
        self.inner.write_all(self.buf.as_slice())?;
        self.pos = u64::try_from(self.buf.len()).unwrap();
        self.finished = true;

        Ok(())
    }
}

impl<W> SdCardWrapper<W>
where
    W: io::Write + io::Seek + Commit,
{
    /// Remove both primary and backup partition metadata before any payload is written.
    ///
    /// The aligned zero block prevents an automounter from rediscovering an old MBR/GPT layout
    /// while the replacement image is still incomplete. Clearing the end also retires a stale GPT
    /// backup header left by the previous contents of a larger card.
    pub(crate) fn hide_layout(&mut self, capacity: Option<u64>) -> io::Result<()> {
        if capacity.is_some_and(|size| size < BLOCK_SIZE as u64) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination is smaller than the deferred layout block",
            ));
        }

        self.inner.rewind()?;
        self.inner.write_all(self.buf.as_slice())?;
        if let Some(end) = capacity.filter(|size| *size >= BLOCK_SIZE as u64) {
            self.inner
                .seek(io::SeekFrom::Start(end - BLOCK_SIZE as u64))?;
            self.inner.write_all(self.buf.as_slice())?;
        }
        self.inner.commit()?;
        self.inner.rewind()?;
        self.pos = 0;
        Ok(())
    }
}

impl<W> Commit for SdCardWrapper<W>
where
    W: Commit,
{
    fn commit(&mut self) -> io::Result<()> {
        self.inner.commit()
    }
}

impl<W> PublishLayout for SdCardWrapper<W>
where
    W: io::Read + io::Write + io::Seek + Commit,
{
    fn publish_layout(&mut self) -> crate::Result<()> {
        self.finish()
            .map_err(|source| crate::Error::SyncFailed { source })?;
        self.inner
            .commit()
            .map_err(|source| crate::Error::SyncFailed { source })?;

        let mut actual = Box::new(DirectIoBuffer::<BLOCK_SIZE>::new());
        self.inner.rewind()?;
        read_at_least(
            &mut self.inner,
            actual.as_mut_slice(),
            self.buf.as_slice().len(),
        )?;

        if actual.as_slice() != self.buf.as_slice() {
            return Err(crate::Error::LayoutReadBackMismatch);
        }

        self.pos = u64::try_from(BLOCK_SIZE).unwrap();
        Ok(())
    }
}

impl<W> io::Read for SdCardWrapper<W>
where
    W: io::Read + io::Seek,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let pos = usize::try_from(self.pos).unwrap();

        let count = if self.is_cached(pos) {
            let count = std::cmp::min(self.buf.len() - pos, buf.len());
            self.inner
                .seek(io::SeekFrom::Current(i64::try_from(count).unwrap()))?;
            buf[..count].copy_from_slice(&self.buf.as_slice()[pos..(pos + count)]);
            count
        } else {
            self.inner.read(buf)?
        };

        self.pos += u64::try_from(count).unwrap();
        Ok(count)
    }
}

impl<W> io::Write for SdCardWrapper<W>
where
    W: io::Write + io::Seek,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let pos = usize::try_from(self.pos).unwrap();

        let count = if self.is_cached(pos) {
            let count = std::cmp::min(self.buf.len() - pos, buf.len());
            self.inner
                .seek(io::SeekFrom::Current(i64::try_from(count).unwrap()))?;
            self.buf.as_mut_slice()[pos..(pos + count)].copy_from_slice(&buf[..count]);
            count
        } else {
            self.inner.write(buf)?
        };

        self.pos += u64::try_from(count).unwrap();
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<W> io::Seek for SdCardWrapper<W>
where
    W: io::Seek,
{
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.pos = self.inner.seek(pos)?;
        Ok(self.pos)
    }
}

impl<W> Eject for SdCardWrapper<W>
where
    W: io::Write + io::Seek + Eject,
{
    fn eject(mut self) -> io::Result<()> {
        self.finish()?;
        self.inner.eject()
    }
}

/// Read exactly `buf.len()` bytes, tolerating a short device only past `required`.
///
/// Direct IO wants aligned, block-multiple reads, so the tail of the read-back asks for more bytes
/// than the image actually occupies. Hitting the end of a plain file there is fine; hitting it
/// before `required` means the device gave back less than was written to it, which is a failure.
pub(crate) fn read_at_least(
    mut src: impl io::Read,
    buf: &mut [u8],
    required: usize,
) -> io::Result<()> {
    let mut pos = 0;

    while pos < buf.len() {
        match src.read(&mut buf[pos..])? {
            0 => break,
            n => pos += n,
        }
    }

    if pos < required {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("device returned {pos} bytes where {required} were written"),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, SeekFrom, Write};

    use crate::helpers::BLOCK_SIZE;

    use super::{Commit, PublishLayout, SdCardWrapper};

    const FILE_LEN: usize = 12 * 1024;

    fn test_data() -> std::io::Cursor<Box<[u8]>> {
        let data: Vec<u8> = (0..FILE_LEN)
            .map(|x| x % 255)
            .map(|x| u8::try_from(x).unwrap())
            .collect();
        std::io::Cursor::new(data.into())
    }

    fn test_file() -> super::DeviceWrapper<std::fs::File> {
        let mut data = test_data();
        let mut f = tempfile::tempfile().unwrap();

        std::io::copy(&mut data, &mut f).unwrap();
        f.rewind().unwrap();

        super::DeviceWrapper::new(f).unwrap()
    }

    #[test]
    fn dev_wrapper_read() {
        let mut temp = test_file();
        let mut buf = [0u8; 50];

        temp.seek(SeekFrom::Start(10)).unwrap();
        temp.read_exact(&mut buf).unwrap();

        let ans: Vec<u8> = (10..60).collect();
        assert_eq!(buf.as_slice(), &ans);

        temp.seek(SeekFrom::Start(4095)).unwrap();
        temp.read_exact(&mut buf).unwrap();

        let ans: Vec<u8> = (4095..4145).map(|x| (x % 255) as u8).collect();
        assert_eq!(buf.as_slice(), &ans);
    }

    #[test]
    fn dev_wrapper_write() {
        let mut temp = test_file();
        let ans = [9u8; 50];

        let mut buf = [9u8; 50];
        temp.seek(SeekFrom::Start(10)).unwrap();
        temp.write_all(&buf).unwrap();

        temp.seek(SeekFrom::Start(4090)).unwrap();
        temp.write_all(&buf).unwrap();

        temp.seek(SeekFrom::Start(10)).unwrap();
        temp.read_exact(&mut buf).unwrap();

        assert_eq!(ans, buf);

        temp.seek(SeekFrom::Start(4090)).unwrap();
        temp.read_exact(&mut buf).unwrap();

        assert_eq!(ans, buf);
    }

    #[test]
    fn sd_card_wrapper() {
        let mut test_data = test_data();
        let mut temp_buf = Vec::with_capacity(FILE_LEN);

        let f = tempfile::tempfile().unwrap();
        f.set_len(FILE_LEN as u64).unwrap();
        let mut sd = SdCardWrapper::new(f);

        std::io::copy(&mut test_data, &mut sd).unwrap();
        sd.flush().unwrap();

        // Read underlying file contents directly
        sd.inner.rewind().unwrap();
        sd.inner.read_to_end(&mut temp_buf).unwrap();

        // Everything after the cached block should already be flushed
        assert_eq!(&test_data.get_ref()[BLOCK_SIZE..], &temp_buf[BLOCK_SIZE..]);
        // First block should still only exist in cache
        assert_eq!(
            &test_data.get_ref()[..BLOCK_SIZE],
            &sd.buf.as_slice()[..BLOCK_SIZE]
        );
        assert!(temp_buf[..BLOCK_SIZE].iter().all(|x| *x == 0));

        temp_buf.clear();

        // Logical reads should still see full data
        sd.rewind().unwrap();
        sd.read_to_end(&mut temp_buf).unwrap();
        assert_eq!(temp_buf.as_slice(), test_data.get_ref().as_ref());

        // A payload commit must not publish the partition table.
        Commit::commit(&mut sd).unwrap();

        temp_buf.clear();
        sd.inner.rewind().unwrap();
        sd.inner.read_to_end(&mut temp_buf).unwrap();
        assert!(temp_buf[..BLOCK_SIZE].iter().all(|x| *x == 0));

        // The explicit final publication writes, syncs and physically reads back the cached block.
        PublishLayout::publish_layout(&mut sd).unwrap();

        temp_buf.clear();

        sd.inner.rewind().unwrap();
        sd.inner.read_to_end(&mut temp_buf).unwrap();
        assert_eq!(temp_buf.as_slice(), test_data.get_ref().as_ref());
    }

    #[derive(Debug)]
    struct CorruptOnCommit(std::io::Cursor<Vec<u8>>);

    impl Read for CorruptOnCommit {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for CorruptOnCommit {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Seek for CorruptOnCommit {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(pos)
        }
    }

    impl Commit for CorruptOnCommit {
        fn commit(&mut self) -> std::io::Result<()> {
            self.0.get_mut()[0] ^= 0xff;
            Ok(())
        }
    }

    #[test]
    fn published_layout_is_read_back_from_the_destination() {
        let backing = CorruptOnCommit(std::io::Cursor::new(vec![0; FILE_LEN]));
        let mut sd = SdCardWrapper::new(backing);
        sd.write_all(&vec![0x5a; BLOCK_SIZE]).unwrap();

        let error = PublishLayout::publish_layout(&mut sd)
            .expect_err("a corrupted physical layout block must fail publication");
        assert!(matches!(error, crate::Error::LayoutReadBackMismatch));
    }

    #[test]
    fn formatter_publishes_an_mbr_with_a_mountable_fat32_partition() {
        const FORMAT_SIZE: u64 = 64 * 1024 * 1024;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("formatted.img");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(FORMAT_SIZE).unwrap();
        drop(file);

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        super::format_device(file, FORMAT_SIZE).unwrap();

        let mut disk = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mbr = mbrman::MBR::read_from(&mut disk, 512).unwrap();
        let partition = mbr.get(1).unwrap();
        assert_eq!(partition.sys, 0x0c);
        assert_eq!(partition.starting_lba, 2048);

        let start = u64::from(partition.starting_lba) * 512;
        let end = start + u64::from(partition.sectors) * 512;
        let slice = fscommon::StreamSlice::new(disk, start, end).unwrap();
        let filesystem =
            fatfs::FileSystem::new(fscommon::BufStream::new(slice), fatfs::FsOptions::new())
                .unwrap();
        assert_eq!(filesystem.fat_type(), fatfs::FatType::Fat32);
    }
}
