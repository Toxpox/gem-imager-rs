use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

type Waiters = HashMap<[u8; 32], Weak<tokio::sync::Mutex<()>>>;

/// Serialises downloads of the same digest.
///
/// Two layers are needed. The in-process map coalesces concurrent callers that share a
/// `Downloader`, and is what keeps a single GUI session from issuing two requests for one image.
/// It cannot see anything outside its own map, so a second `Downloader` value, or a second CLI
/// process pointed at the same cache, still downloaded the same multi-gigabyte image again. The
/// lock file closes that gap for every user of the same cache directory.
#[derive(Debug, Clone, Default)]
pub(crate) struct SingleFlight {
    inner: Arc<Mutex<Waiters>>,
}

/// Held for the duration of one download. Releases the cross-process lock on drop.
#[derive(Debug)]
pub(crate) struct Slot {
    _in_process: tokio::sync::OwnedMutexGuard<()>,
    // The lock is released when the file handle closes, so this is kept alive deliberately.
    cross_process: Option<File>,
    lock_path: Option<PathBuf>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(file) = self.cross_process.take() {
            let _ = file.unlock();
            drop(file);
        }
        // Best effort: another process may already hold the same lock file, in which case the
        // unlink loses a harmless race and the file is reused.
        if let Some(path) = self.lock_path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

impl SingleFlight {
    /// Acquires the download slot for `key`, waiting for any other holder in this process and
    /// then for any other process using `cache_dir`.
    pub(crate) async fn acquire(&self, key: [u8; 32], cache_dir: &Path) -> Slot {
        let lock = {
            let mut map = self
                .inner
                .lock()
                .expect("single-flight map is never poisoned");

            match map.get(&key).and_then(Weak::upgrade) {
                Some(existing) => existing,
                None => {
                    map.retain(|_, waiter| waiter.strong_count() > 0);
                    let fresh = Arc::new(tokio::sync::Mutex::new(()));
                    map.insert(key, Arc::downgrade(&fresh));
                    fresh
                }
            }
        };

        let in_process = lock.lock_owned().await;

        let lock_path = cache_dir.join(format!(".lock-{}", const_hex::encode(key)));
        // A cache directory that cannot hold a lock file (read-only, or a filesystem without
        // advisory locking) must not stop the download. The in-process guard still applies and
        // the digest check still protects correctness; only cross-process coalescing is lost.
        let cross_process = match Self::lock_file(&lock_path).await {
            Ok(file) => Some(file),
            Err(err) => {
                tracing::debug!(
                    "Cross-process download coordination is unavailable in {}: {err}",
                    cache_dir.display()
                );
                None
            }
        };

        Slot {
            _in_process: in_process,
            lock_path: cross_process.as_ref().map(|_| lock_path),
            cross_process,
        }
    }

    /// Blocking `flock` moved off the runtime: waiting for another process can take as long as
    /// that process's download, which would stall every other task on this worker thread.
    async fn lock_file(path: &Path) -> io::Result<File> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                // The file is only ever a lock token; its contents are never read or written,
                // and truncating would race with another process holding the same handle.
                .truncate(false)
                .open(&path)?;
            file.lock()?;
            Ok(file)
        })
        .await
        .map_err(io::Error::other)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn the_same_key_is_serialized_and_a_different_key_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let flight = SingleFlight::default();
        let key = [7u8; 32];

        let first = flight.acquire(key, dir.path()).await;

        let other = flight.acquire([9u8; 32], dir.path()).await;
        drop(other);

        let entered = Arc::new(AtomicUsize::new(0));
        let waiter = {
            let flight = flight.clone();
            let entered = entered.clone();
            let dir = dir.path().to_path_buf();
            tokio::spawn(async move {
                let guard = flight.acquire(key, &dir).await;
                entered.fetch_add(1, Ordering::SeqCst);
                drop(guard);
            })
        };

        tokio::task::yield_now().await;
        assert_eq!(
            entered.load(Ordering::SeqCst),
            0,
            "second acquire of the same key must wait"
        );

        drop(first);
        waiter.await.unwrap();
        assert_eq!(entered.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_released_key_does_not_leak_a_map_entry() {
        let dir = tempfile::tempdir().unwrap();
        let flight = SingleFlight::default();

        for i in 0..8u8 {
            drop(flight.acquire([i; 32], dir.path()).await);
        }

        let live = flight.inner.lock().unwrap().len();
        assert!(live <= 1, "expected reclaimed slots, found {live}");
    }

    #[tokio::test]
    async fn separate_instances_sharing_a_cache_are_serialized() {
        let dir = tempfile::tempdir().unwrap();
        let key = [3u8; 32];

        // Two independent SingleFlight values stand in for two Downloader instances, which is
        // exactly the case the in-process map cannot see.
        let one = SingleFlight::default();
        let two = SingleFlight::default();

        let held = one.acquire(key, dir.path()).await;

        let entered = Arc::new(AtomicUsize::new(0));
        let waiter = {
            let entered = entered.clone();
            let dir = dir.path().to_path_buf();
            tokio::spawn(async move {
                let guard = two.acquire(key, &dir).await;
                entered.fetch_add(1, Ordering::SeqCst);
                drop(guard);
            })
        };

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(
            entered.load(Ordering::SeqCst),
            0,
            "a second instance must wait on the cross-process lock"
        );

        drop(held);
        tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("the waiter must proceed once the lock is released")
            .unwrap();
        assert_eq!(entered.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_keys_do_not_block_across_instances() {
        let dir = tempfile::tempdir().unwrap();
        let one = SingleFlight::default();
        let two = SingleFlight::default();

        let held = one.acquire([1u8; 32], dir.path()).await;
        let other = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            two.acquire([2u8; 32], dir.path()),
        )
        .await
        .expect("a different digest must not wait");
        drop(other);
        drop(held);
    }

    #[tokio::test]
    async fn a_cache_directory_without_lock_support_still_downloads() {
        // A path that cannot be created as a file: acquiring must degrade instead of failing.
        let missing = Path::new("/nonexistent-cache-dir-for-single-flight-test");
        let flight = SingleFlight::default();
        let slot = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            flight.acquire([5u8; 32], missing),
        )
        .await
        .expect("acquire must not hang when locking is unavailable");
        assert!(
            slot.cross_process.is_none(),
            "no cross-process lock should be held"
        );
    }
}
