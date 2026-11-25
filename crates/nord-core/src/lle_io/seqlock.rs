//! Generic single-writer / many-reader seqlock over `AtomicU32` words.
//!
//! Same scheme as `engine::telemetry_cell::TelemetryCell` (spec: `docs/specs/telemetry.md`), but
//! sized by a const generic so the LLE telemetry (OLED frames, LEDs) can reuse it. The payload is
//! atomics, so there is no data race and no `unsafe`. Neither side locks, waits unboundedly or
//! allocates; the cell is built once at init.

use std::sync::atomic::{fence, AtomicU32, Ordering};

/// Reader retry budget before giving up for this call.
/// ASSUMED: carried over from `TelemetryCell` (34 words), where it never starved. This cell can be
/// ~530 words (LLE telemetry), so a write window is ~15x longer and a reader may overlap it more
/// often: [`SeqCell::read`] may return `false` under load, and callers must keep the previous
/// snapshot and retry next frame. On a loaded Windows CI runner a 300 ms stress test still got
/// hundreds of consistent reads (PR #650 run, 749), so starvation is rare, not impossible.
const MAX_ATTEMPTS: usize = 64;

/// Seqlock cell of `W` words. Exactly one thread may call [`Self::publish`].
#[derive(Debug)]
pub struct SeqCell<const W: usize> {
    seq: AtomicU32,
    words: [AtomicU32; W],
}

impl<const W: usize> Default for SeqCell<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const W: usize> SeqCell<W> {
    /// Empty cell: [`Self::read`] fails until the first publish.
    pub fn new() -> Self {
        Self {
            seq: AtomicU32::new(0),
            words: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }

    /// Stores `src` (writer thread only). Wait-free, allocation-free.
    pub fn publish(&self, src: &[u32; W]) {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed); // odd: write in progress
        fence(Ordering::Release);
        for (slot, w) in self.words.iter().zip(src) {
            slot.store(*w, Ordering::Relaxed);
        }
        self.seq.store(s.wrapping_add(2), Ordering::Release); // even: stable
    }

    /// Number of completed publishes. The sequence is a `u32` counted twice per publish, so this
    /// wraps after 2^31 publishes (about 4 years at 60 per second); a publish whose
    /// sequence lands exactly on `0` reads as "never published" until the next publish.
    pub fn publishes(&self) -> u32 {
        self.seq.load(Ordering::Relaxed) / 2
    }

    /// Copies the newest consistent snapshot into `dst`. `false` before the first publish or after
    /// `MAX_ATTEMPTS` overlaps with the writer; `dst` is then unspecified.
    pub fn read(&self, dst: &mut [u32; W]) -> bool {
        for _ in 0..MAX_ATTEMPTS {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 == 0 {
                return false;
            }
            if s1 & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            for (d, slot) in dst.iter_mut().zip(&self.words) {
                *d = slot.load(Ordering::Relaxed);
            }
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn empty_cell_fails_then_returns_published_words() {
        let cell = SeqCell::<4>::new();
        let mut out = [0u32; 4];
        assert!(!cell.read(&mut out));
        cell.publish(&[1, 2, 3, 4]);
        assert!(cell.read(&mut out));
        assert_eq!(out, [1, 2, 3, 4]);
        cell.publish(&[5, 6, 7, 8]);
        assert!(cell.read(&mut out));
        assert_eq!(out, [5, 6, 7, 8]);
        assert_eq!(cell.publishes(), 2);
    }

    /// Writes made by the concurrent test's writer thread (bounded, so the test ends on its own).
    const WRITES: u32 = 4_000;

    /// One writer republishes `[n; 32]` a fixed number of times, yielding every 64 writes; readers
    /// loop until the writer is done and check every word is equal. Pass conditions do not depend on
    /// scheduling: each successful read is untorn, and once writes stop a read must succeed and
    /// return the last value (sequence is even and stable, so no retry budget can run out).
    #[test]
    fn concurrent_readers_never_see_a_torn_snapshot() {
        let cell = Arc::new(SeqCell::<32>::new());
        let done = Arc::new(AtomicBool::new(false));
        let writer = {
            let (cell, done) = (Arc::clone(&cell), Arc::clone(&done));
            std::thread::spawn(move || {
                for n in 1..=WRITES {
                    cell.publish(&[n; 32]);
                    if n % 64 == 0 {
                        std::thread::yield_now(); // let readers in; bounded, so no scheduling dependence
                    }
                }
                done.store(true, Ordering::Release);
            })
        };
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let (cell, done) = (Arc::clone(&cell), Arc::clone(&done));
                std::thread::spawn(move || {
                    let mut out = [0u32; 32];
                    let mut last = 0u32;
                    while !done.load(Ordering::Acquire) {
                        if cell.read(&mut out) {
                            assert!(out.iter().all(|&w| w == out[0]), "torn: {out:?}");
                            assert!(out[0] >= last, "went backwards: {} < {last}", out[0]);
                            last = out[0];
                        } else {
                            std::thread::yield_now();
                        }
                    }
                    // Writer finished: the cell is stable, so this read cannot fail.
                    assert!(
                        cell.read(&mut out),
                        "no consistent read after writes stopped"
                    );
                    assert_eq!(out, [WRITES; 32]);
                })
            })
            .collect();
        for r in readers {
            r.join().unwrap();
        }
        writer.join().unwrap();
        assert_eq!(cell.publishes(), WRITES);
    }

    /// Deterministic torn-read check without threads: a manual odd/even sequence stands in for the
    /// writer being mid-publish.
    #[test]
    fn read_rejects_in_progress_write_and_accepts_after_completion() {
        let cell = SeqCell::<4>::new();
        cell.publish(&[7; 4]);
        // Begin a publish by hand: odd sequence, half the words new.
        cell.seq
            .store(cell.seq.load(Ordering::Relaxed) + 1, Ordering::Release);
        cell.words[0].store(9, Ordering::Relaxed);
        cell.words[1].store(9, Ordering::Relaxed);
        let mut out = [0u32; 4];
        assert!(
            !cell.read(&mut out),
            "must not return a half-written snapshot"
        );
        // Finish it: even sequence, all words new.
        cell.words[2].store(9, Ordering::Relaxed);
        cell.words[3].store(9, Ordering::Relaxed);
        cell.seq
            .store(cell.seq.load(Ordering::Relaxed) + 1, Ordering::Release);
        assert!(cell.read(&mut out));
        assert_eq!(out, [9; 4]);
    }
}
