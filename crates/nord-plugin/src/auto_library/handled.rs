//! What the library worker has finished with, for [`super::LibraryLink::wait_handled`].
//!
//! A last-handled record (generation and value) behind a condition variable replaces the bounded
//! "done" queue of the first version: that queue was drained only by `wait_handled`, so after
//! eight or more program changes the later signals were dropped and a later `initialize` waited
//! out its whole timeout (or returned early on a stale equal value). A record cannot overflow and
//! a generation separates an old handling of the same value from a new one.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Last {
    /// Generation of the newest request the worker handled (0 = none yet).
    generation: u64,
    /// Its parameter value.
    value: Option<i32>,
}

/// Shared between the link (waits) and the worker (records).
#[derive(Default)]
pub(super) struct Handled {
    last: Mutex<Last>,
    changed: Condvar,
}

impl Handled {
    /// Worker: request `generation` (value `value`) is handled (loaded, skipped or failed).
    pub(super) fn record(&self, generation: u64, value: i32) {
        if let Ok(mut last) = self.last.lock() {
            last.generation = generation;
            last.value = Some(value);
        }
        self.changed.notify_all();
    }

    /// Blocks until the worker has handled request `generation` or a later one, and that was for
    /// `value`; `false` after `timeout`. Never call it from the audio thread.
    pub(super) fn wait(&self, generation: u64, value: i32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let Ok(mut last) = self.last.lock() else {
            return false;
        };
        loop {
            if last.generation >= generation && last.value == Some(value) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            match self.changed.wait_timeout(last, deadline - now) {
                Ok((guard, _)) => last = guard,
                Err(_) => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_record_satisfies_the_same_or_an_older_generation_only_for_its_value() {
        let h = Handled::default();
        assert!(!h.wait(0, 1, Duration::from_millis(20)), "nothing recorded");
        h.record(3, 1);
        assert!(h.wait(3, 1, Duration::from_millis(20)));
        assert!(h.wait(1, 1, Duration::from_millis(20)));
        assert!(!h.wait(4, 1, Duration::from_millis(20)), "newer request");
        assert!(!h.wait(3, 2, Duration::from_millis(20)), "other value");
    }

    #[test]
    fn a_wait_wakes_when_the_worker_records_and_never_overflows() {
        let h = Arc::new(Handled::default());
        for g in 1..=100 {
            h.record(g, 1);
        }
        let h2 = Arc::clone(&h);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            h2.record(101, 2);
        });
        let start = Instant::now();
        assert!(h.wait(101, 2, Duration::from_secs(5)));
        assert!(start.elapsed() < Duration::from_secs(3));
        t.join().unwrap();
    }
}
