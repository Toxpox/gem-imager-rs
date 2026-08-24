
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

type Waiters = HashMap<[u8; 32], Weak<tokio::sync::Mutex<()>>>;

#[derive(Debug, Clone, Default)]
pub(crate) struct SingleFlight {
    inner: Arc<Mutex<Waiters>>,
}

impl SingleFlight {
    pub(crate) async fn acquire(&self, key: [u8; 32]) -> tokio::sync::OwnedMutexGuard<()> {
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

        lock.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn the_same_key_is_serialized_and_a_different_key_is_not() {
        let flight = SingleFlight::default();
        let key = [7u8; 32];

        let first = flight.acquire(key).await;

        let other = flight.acquire([9u8; 32]).await;
        drop(other);

        let entered = Arc::new(AtomicUsize::new(0));
        let waiter = {
            let flight = flight.clone();
            let entered = entered.clone();
            tokio::spawn(async move {
                let guard = flight.acquire(key).await;
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
        let flight = SingleFlight::default();

        for i in 0..8u8 {
            drop(flight.acquire([i; 32]).await);
        }

        let live = flight.inner.lock().unwrap().len();
        assert!(live <= 1, "expected reclaimed slots, found {live}");
    }
}
