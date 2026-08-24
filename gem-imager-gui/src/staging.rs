use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const HEADROOM: u64 = 256 * 1024 * 1024;

const PREFIX: &str = "t3-staging-";

static STAGING_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub(crate) enum StagingError {
    #[cfg_attr(test, allow(dead_code))]
    #[error("no application cache directory is available for image staging")]
    NoCacheDir,
    #[error("failed to prepare the image staging directory: {source}")]
    Io {
        #[source]
        source: io::Error,
    },
    #[error(
        "not enough free space for image staging: {required} bytes required, \
         {available} bytes available"
    )]
    InsufficientSpace { required: u64, available: u64 },
}

#[cfg(not(test))]
pub(crate) fn staging_dir() -> Result<PathBuf, StagingError> {
    let dirs = crate::helpers::project_dirs().ok_or(StagingError::NoCacheDir)?;
    Ok(dirs.cache_dir().join("dfu-staging"))
}

#[cfg(test)]
pub(crate) fn staging_dir() -> Result<PathBuf, StagingError> {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    Ok(std::env::current_dir()
        .map_err(|source| StagingError::Io { source })?
        .join("target")
        .join("test-cache")
        .join(format!(
            "dfu-staging-{}-{}",
            std::process::id(),
            hasher.finish()
        )))
}

#[derive(Debug)]
pub(crate) struct StagingImage {
    path: PathBuf,
}

impl StagingImage {
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

        let path = dir.join(format!(
            "{PREFIX}{}-{}.img",
            std::process::id(),
            STAGING_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StagingImage {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => tracing::info!("Removed the staging image"),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("Failed to remove the image staging file: {e}"),
        }
    }
}

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
            Ok(()) => tracing::info!("Removed a stale staging image from a previous run"),
            Err(e) => tracing::warn!("Failed to remove a stale image staging file: {e}"),
        }
    }
}

#[cfg(windows)]
fn available_space(dir: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt as _;

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

    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_space_reports_a_real_figure_for_the_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let free = available_space(dir.path()).unwrap();
        assert!(free > 1024 * 1024, "implausible free space: {free}");
    }

    #[test]
    fn a_request_larger_than_the_disk_is_refused_before_any_download() {
        let err = StagingImage::create(u64::MAX - HEADROOM).unwrap_err();
        assert!(
            matches!(err, StagingError::InsufficientSpace { required, .. } if required == u64::MAX),
            "expected an insufficient-space refusal, got {err}"
        );
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

    #[test]
    fn concurrent_staging_images_never_share_a_path() {
        let first = StagingImage::create(0).unwrap();
        let second = StagingImage::create(0).unwrap();
        assert_ne!(first.path(), second.path());
    }

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
