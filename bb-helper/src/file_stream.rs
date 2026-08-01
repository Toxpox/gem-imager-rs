//! A data stream with sync Read and async Write halves. Has a backing file.
//!
//! This is designed to be used for large data streams that cannot live in memory.

use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::io::{AsyncSeekExt, AsyncWriteExt};

type SharedState = Arc<(Mutex<bool>, Condvar)>;

/// Distinguishes the scratch files of concurrent [`WriterFileStream::persist`] calls that target
/// the same final path.
static PERSIST_NONCE: AtomicU64 = AtomicU64::new(0);

/// Removes a half-written scratch file unless it was published.
///
/// the transport policy forbids a cancelled or partial download from being reachable under the
/// final cache name; the scratch file must not survive as litter either.
struct ScratchFile(Option<PathBuf>);

impl ScratchFile {
    fn new(path: PathBuf) -> Self {
        Self(Some(path))
    }

    fn path(&self) -> &Path {
        self.0
            .as_deref()
            .expect("scratch path is taken only when publishing or dropping")
    }

    /// Give up ownership after a successful rename, so the file is not deleted.
    fn published(mut self) {
        self.0 = None;
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Asynchronous writer half of a file-backed stream.
///
/// Writes data asynchronously to a temporary file. The data can be persisted
/// to a permanent location using [`persist`](Self::persist).
pub struct WriterFileStream {
    file: tokio::fs::File,
    writing: SharedState,
}

impl WriterFileStream {
    const fn new(file: tokio::fs::File, writing: SharedState) -> Self {
        Self { file, writing }
    }

    /// Publishes the written data at `path`, atomically.
    ///
    /// The copy goes to a scratch file next to `path` — same directory, therefore same filesystem,
    /// so the final step is a rename and never a partial copy. The scratch file is flushed and
    /// `fsync`ed before the rename, so a crash cannot leave `path` naming a file whose contents
    /// were still in the page cache. If anything fails, `path` keeps whatever it held before and
    /// the scratch file is removed (the transport policy).
    pub async fn persist(&mut self, path: &Path) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "persist target must have a parent directory",
            )
        })?;
        let file_name = path.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "persist target must name a file",
            )
        })?;

        let scratch = ScratchFile::new(parent.join(format!(
            "{}.part-{}-{}",
            file_name.to_string_lossy(),
            std::process::id(),
            PERSIST_NONCE.fetch_add(1, Ordering::Relaxed)
        )));

        {
            let mut f = tokio::fs::File::create(scratch.path()).await?;
            self.file.seek(io::SeekFrom::Start(0)).await?;

            tokio::io::copy(&mut self.file, &mut f).await?;

            // Causes errors if not present
            f.flush().await?;
            // Rename only publishes the directory entry; without this the bytes themselves are
            // not guaranteed to have reached the device.
            f.sync_all().await?;
        }

        tokio::fs::rename(scratch.path(), path).await?;
        scratch.published();

        Ok(())
    }
}

impl tokio::io::AsyncWrite for WriterFileStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, io::Error>> {
        let res = std::pin::Pin::new(&mut self.file).poll_write(cx, buf);
        self.writing.1.notify_all();
        res
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        let res = std::pin::Pin::new(&mut self.file).poll_flush(cx);
        self.writing.1.notify_all();
        res
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        std::pin::Pin::new(&mut self.file).poll_shutdown(cx)
    }
}

impl Drop for WriterFileStream {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.writing;
        let mut writing = lock.lock().unwrap();
        *writing = false;
        cvar.notify_all();
    }
}

/// Synchronous reader half of a file-backed stream.
///
/// Reads data from the same temporary file as the writer. While the writer
/// is active, reading will block until data is available or the writer closes.
pub struct ReaderFileStream {
    file: std::fs::File,
    writing: SharedState,
}

impl ReaderFileStream {
    const fn new(file: std::fs::File, writing: SharedState) -> Self {
        Self { file, writing }
    }
}

impl std::io::Read for ReaderFileStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let count = self.file.read(buf)?;

            if count == 0 {
                let (lock, cvar) = &*self.writing;
                let writing = lock.lock().unwrap();

                if *writing {
                    drop(cvar.wait(writing));
                    continue;
                }
            }

            return Ok(count);
        }
    }
}

// ReaderFileStream behaves like a stream backed by a growing file rather than
// a normal fully materialized file.
//
// Seeking within the currently written region works normally. However:
//
// - Seeking beyond the current file length waits until the writer produces
//   enough data or closes.
// - SeekFrom::End is unsupported while the writer is still active because the
//   final file length is not yet known, so offsets relative to the end cannot
//   be resolved correctly.
// - Once the writer is dropped, seek behavior matches normal file semantics.
impl std::io::Seek for ReaderFileStream {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        loop {
            let (lock, cvar) = &*self.writing;
            let writing = lock.lock().unwrap();

            // If writing done, use normal file seek.
            if !*writing {
                return self.file.seek(pos);
            }

            let len = self.file.metadata()?.len();
            let target = match pos {
                io::SeekFrom::Start(x) => x,
                io::SeekFrom::End(_) => {
                    // We don't know true file len yet. So just return
                    // unsupported. Not sure if we should just wait for writing to finish.
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Seek from end is unsupported",
                    ));
                }
                io::SeekFrom::Current(x) => self
                    .file
                    .stream_position()?
                    .checked_add_signed(x)
                    .ok_or(io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"))?,
            };

            if target <= len {
                return self.file.seek(pos);
            }
            drop(cvar.wait(writing));
        }
    }
}

/// Creates a new file-backed stream with separate reader and writer halves.
///
/// Returns a tuple of (writer, reader) that share a temporary file.
/// The writer can write asynchronously, while the reader provides synchronous access.
pub fn file_stream() -> io::Result<(WriterFileStream, ReaderFileStream)> {
    let file = tempfile::NamedTempFile::new()?;
    let flag = Arc::new((Mutex::new(true), Condvar::new()));

    let reader = ReaderFileStream::new(file.reopen()?, flag.clone());
    let writer = WriterFileStream::new(file.into_file().into(), flag);

    Ok((writer, reader))
}
