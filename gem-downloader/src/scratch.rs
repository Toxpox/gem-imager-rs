use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// A scratch file that is removed when it goes out of scope.
///
/// `tempfile`'s own guard cannot be used here: the file has to survive being handed to
/// `tokio::fs::rename`, so ownership of the path is taken away from `TempPath`. Doing that with
/// `.keep()` disabled cleanup entirely, and every path that did not return a plain `Err` (task
/// cancellation, a failed publish, a panic) left a multi-gigabyte `.part-*` file behind in the
/// user's cache. This guard restores cleanup for all of them and is disarmed only once the file
/// has actually been published under its final name.
#[derive(Debug)]
pub(crate) struct ScratchFile {
    path: Option<PathBuf>,
}

impl ScratchFile {
    /// Creates a uniquely named scratch file inside `dir`.
    pub(crate) fn create_in(dir: &Path) -> io::Result<Self> {
        let path = tempfile::Builder::new()
            .prefix(".part-")
            .tempfile_in(dir)?
            .into_temp_path()
            .keep()
            .map_err(|err| err.error)?;

        Ok(Self { path: Some(path) })
    }

    pub(crate) fn path(&self) -> &Path {
        self.path
            .as_deref()
            .expect("scratch path is taken only on disarm, which consumes the guard")
    }

    /// Gives up ownership of the path, so dropping the guard no longer deletes the file.
    /// Used once the scratch file has been renamed into place.
    pub(crate) fn disarm(mut self) -> PathBuf {
        self.path.take().expect("disarm consumes the guard")
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            // Blocking removal is deliberate. An async delete cannot be awaited from `drop`, so a
            // spawned cleanup task would be lost exactly in the cancellation case this exists for.
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_the_guard_removes_the_scratch_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = {
            let scratch = ScratchFile::create_in(dir.path()).unwrap();
            let path = scratch.path().to_path_buf();
            assert!(path.exists(), "scratch file should exist while held");
            path
        };
        assert!(!path.exists(), "scratch file must be removed on drop");
    }

    #[test]
    fn a_disarmed_guard_leaves_the_file_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = ScratchFile::create_in(dir.path()).unwrap();
        let path = scratch.disarm();
        assert!(path.exists(), "a disarmed guard must not delete the file");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_panic_while_holding_the_guard_still_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let recorded = std::panic::catch_unwind(|| {
            let scratch = ScratchFile::create_in(dir.path()).unwrap();
            let path = scratch.path().to_path_buf();
            std::panic::panic_any(path);
        });

        let path = *recorded
            .expect_err("the closure panics")
            .downcast::<PathBuf>()
            .expect("the panic payload is the scratch path");
        assert!(!path.exists(), "unwinding must remove the scratch file");
    }
}
