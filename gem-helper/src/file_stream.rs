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

static PERSIST_NONCE: AtomicU64 = AtomicU64::new(0);

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

pub struct WriterFileStream {
    file: tokio::fs::File,
    writing: SharedState,
}

impl WriterFileStream {
    const fn new(file: tokio::fs::File, writing: SharedState) -> Self {
        Self { file, writing }
    }

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

            f.flush().await?;
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

impl std::io::Seek for ReaderFileStream {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        loop {
            let (lock, cvar) = &*self.writing;
            let writing = lock.lock().unwrap();

            if !*writing {
                return self.file.seek(pos);
            }

            let len = self.file.metadata()?.len();
            let target = match pos {
                io::SeekFrom::Start(x) => x,
                io::SeekFrom::End(_) => {
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

pub fn file_stream() -> io::Result<(WriterFileStream, ReaderFileStream)> {
    let file = tempfile::NamedTempFile::new()?;
    let flag = Arc::new((Mutex::new(true), Condvar::new()));

    let reader = ReaderFileStream::new(file.reopen()?, flag.clone());
    let writer = WriterFileStream::new(file.into_file().into(), flag);

    Ok((writer, reader))
}
