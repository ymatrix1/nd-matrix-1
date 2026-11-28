//! One decode per process of each piano / sample bank of the library (`auto_library.rs`): a host
//! makes several instances of the plugin (a bounce in SoundCraft builds three), and a factory
//! grand is 218 MB in the archive. Decoded banks are shared through `Weak` references, so they
//! live exactly as long as an engine or a worker holds them, and a bank being decoded by one
//! instance is waited for by the others instead of decoded again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

/// Per-key slot: the lock serialises the decode, the `Weak` finds a live copy.
type Slot<T> = Arc<Mutex<Weak<T>>>;

/// A process-wide registry of decoded values by key.
pub struct Shared<T> {
    /// Built on first use (`HashMap::new` is not `const`).
    slots: Mutex<Option<HashMap<String, Slot<T>>>>,
}

impl<T> Default for Shared<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Shared<T> {
    pub const fn new() -> Self {
        Self {
            slots: Mutex::new(None),
        }
    }

    /// The live value for `key`, else the result of `decode` (run at most once at a time per key;
    /// a second caller waits and gets the first one's value). A failed decode is not cached.
    pub fn get_or_decode(
        &self,
        key: &str,
        decode: impl FnOnce() -> Result<T, String>,
    ) -> Result<Arc<T>, String> {
        let slot = {
            let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
            let slots = slots.get_or_insert_with(HashMap::new);
            Arc::clone(slots.entry(key.to_string()).or_default())
        };
        let mut weak = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(live) = weak.upgrade() {
            return Ok(live);
        }
        let value = Arc::new(decode()?);
        *weak = Arc::downgrade(&value);
        Ok(value)
    }

    /// Keys with a live value (for tests and diagnostics).
    pub fn live(&self) -> usize {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots
            .iter()
            .flat_map(HashMap::values)
            .filter(|s| {
                s.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .upgrade()
                    .is_some()
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn the_same_key_is_decoded_once_while_a_copy_lives_and_again_after_it_is_gone() {
        let shared: Shared<Vec<u8>> = Shared::new();
        let decodes = AtomicUsize::new(0);
        let decode = || {
            decodes.fetch_add(1, Ordering::SeqCst);
            Ok(vec![1, 2, 3])
        };
        let a = shared.get_or_decode("k", decode).unwrap();
        let b = shared.get_or_decode("k", decode).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
        assert_eq!(shared.live(), 1);
        drop(a);
        drop(b);
        assert_eq!(shared.live(), 0);
        let c = shared.get_or_decode("k", decode).unwrap();
        assert_eq!(*c, vec![1, 2, 3]);
        assert_eq!(decodes.load(Ordering::SeqCst), 2);
        // Another key is its own decode; a failure caches nothing.
        let _d = shared.get_or_decode("other", decode).unwrap();
        assert_eq!(decodes.load(Ordering::SeqCst), 3);
        assert!(shared
            .get_or_decode("bad", || Err::<Vec<u8>, _>("no".to_string()))
            .is_err());
        assert_eq!(shared.live(), 2);
    }

    #[test]
    fn concurrent_requests_for_one_key_share_the_first_decode() {
        static SHARED: Shared<u64> = Shared::new();
        let decodes = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let decodes = Arc::clone(&decodes);
                std::thread::spawn(move || {
                    SHARED
                        .get_or_decode("slow", || {
                            decodes.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            Ok(7)
                        })
                        .unwrap()
                })
            })
            .collect();
        let values: Vec<Arc<u64>> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(values.iter().all(|v| Arc::ptr_eq(v, &values[0])));
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
    }
}
