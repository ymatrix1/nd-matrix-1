//! Where the audio thread puts what it must not free (TD-12 remainder, TD-18).
//!
//! `LoadProgram` carries a `Box<ProgramDecoded>` (strings, vectors) and `LoadPianoSound` /
//! `LoadSynthSample` replace an `Arc` whose last owner may free tens of megabytes of samples. Freeing
//! is a heap operation (it can wait for the heap lock another thread holds, and a big block goes
//! back to the OS), so the callback hands the spent value to a reaper thread through a bounded queue
//! (`try_send`: no allocation, never blocks). A full queue leaks the value instead of freeing it
//! on the audio thread; the queue holds [`CAPACITY`] items and the reaper drains it at once.

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use nord_core::cbin::ProgramDecoded;
use nord_core::piano::NpnoSound;
use nord_core::synth::SynthSampleSound;
use std::sync::Arc;

/// Items the queue holds before the audio thread leaks instead of freeing.
///
/// ASSUMED: 64 = several times the largest burst seen (a program load puts one item, a piano or
/// synth sample load one each, and the reaper drains within a scheduler tick). Confirm by counting
/// leaks (none expected) under `ns4 bench --gui --reload N`; raise if a leak is ever reported.
pub const CAPACITY: usize = 64;

/// A value the audio thread is done with.
pub enum Garbage {
    Program(Box<ProgramDecoded>),
    Piano(Option<Arc<NpnoSound>>),
    Synth(Option<Arc<SynthSampleSound>>),
}

/// The audio thread's end of the garbage queue. `Bin::inline()` frees on the spot (tests, headless).
#[derive(Clone, Default)]
pub struct Bin {
    tx: Option<Sender<Garbage>>,
}

impl Bin {
    /// A bin that drops values where they are (no reaper).
    pub fn inline() -> Self {
        Self::default()
    }

    /// Queues `g` for the reaper; leaks it when the queue is full (never frees here).
    pub fn put(&self, g: Garbage) {
        match &self.tx {
            Some(tx) => {
                if let Err(TrySendError::Full(g) | TrySendError::Disconnected(g)) = tx.try_send(g) {
                    std::mem::forget(g);
                }
            }
            None => drop(g),
        }
    }
}

/// A bin plus the receiver a reaper (or a test) drains.
pub fn channel() -> (Bin, Receiver<Garbage>) {
    let (tx, rx) = bounded(CAPACITY);
    (Bin { tx: Some(tx) }, rx)
}

/// A bin whose values a background thread frees as they arrive.
pub fn spawn_reaper() -> Bin {
    let (bin, rx) = channel();
    let spawned = std::thread::Builder::new()
        .name("ns4-reaper".into())
        .spawn(move || {
            for g in rx {
                drop(g);
            }
        });
    match spawned {
        Ok(_) => bin,
        // No thread: the queue would fill up and leak; free inline instead.
        Err(_) => Bin::inline(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_bin_hands_values_over_and_an_inline_bin_drops_them() {
        let (bin, rx) = channel();
        bin.put(Garbage::Program(Box::default()));
        bin.put(Garbage::Piano(None));
        assert_eq!(rx.try_iter().count(), 2);
        Bin::inline().put(Garbage::Synth(None));
    }

    #[test]
    fn a_full_or_closed_queue_leaks_instead_of_freeing() {
        let (bin, rx) = channel();
        for _ in 0..CAPACITY + 3 {
            bin.put(Garbage::Program(Box::default()));
        }
        assert_eq!(rx.try_iter().count(), CAPACITY);
        drop(rx);
        bin.put(Garbage::Program(Box::default()));
    }

    #[test]
    fn the_reaper_thread_accepts_values() {
        let bin = spawn_reaper();
        for _ in 0..10 {
            bin.put(Garbage::Program(Box::default()));
        }
    }
}
