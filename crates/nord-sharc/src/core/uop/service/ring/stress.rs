//! Seeded two-thread stress of the ring: one producer and one consumer move a long sequence
//! through a few slots with pauses chosen by a fixed seed.
//!
//! OWNER: W1-SVC. The thread schedule is not reproducible, but the sequence, the payloads and the
//! pause pattern each side draws are (a pure function of [`SEED`]), and what is asserted does not
//! depend on the schedule: every item arrives once, in order, with its payload, and is dropped
//! exactly once. The run is not vacuous: a handshake makes the consumer check the empty ring
//! before the producer starts and hold off until the producer has seen a full ring, so "empty",
//! "full" and the wrap (the sequence is far longer than the ring) all happen in every run.
use super::*;
use std::sync::atomic::AtomicBool;

/// Fixed seed of every run. ASSUMED arbitrary: any nonzero value exercises the same code.
const SEED: u64 = 0x5eed_0f5e_c0de;
/// Items per run. ASSUMED enough to wrap the smallest ring thousands of times and still run in
/// well under a second in the debug profile.
const ITEMS: u64 = 20_000;

/// xorshift64* (Vigna 2014): a tiny deterministic generator, local so no other package's test
/// support is a dependency.
struct Xs(u64);

impl Xs {
    fn new(seed: u64) -> Self {
        Xs(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A pause drawn from the stream: usually none, sometimes a short spin or a yield.
    fn pause(&mut self) {
        match self.next() % 8 {
            6 => (0..self.next() % 64).for_each(|_| std::hint::spin_loop()),
            7 => std::thread::yield_now(),
            _ => {}
        }
    }
}

/// The payload of item `seq`: a pure function of the seed and the position.
fn tag_of(seed: u64, seq: u64) -> u64 {
    Xs::new(seed ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15)).next()
}

/// One queued item: its position, a seeded payload and a drop counter.
struct Item {
    seq: u64,
    tag: u64,
    drops: Arc<AtomicUsize>,
}

impl Drop for Item {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

/// What a run saw.
struct Report {
    sum: u64,
    full_hits: u64,
    drops: usize,
}

/// Moves [`ITEMS`] items from `push` to `pop` on two threads and returns what happened.
fn stress(
    seed: u64,
    mut push: impl FnMut(Item) -> Result<(), Item> + Send,
    mut pop: impl FnMut() -> Option<Item> + Send,
) -> Report {
    let drops = Arc::new(AtomicUsize::new(0));
    let (ready, full_seen) = (AtomicBool::new(false), AtomicBool::new(false));
    let (ready, full_seen) = (&ready, &full_seen);
    let producer_drops = Arc::clone(&drops);
    let (full_hits, sum) = std::thread::scope(|s| {
        let producer = s.spawn(move || {
            while !ready.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            let mut pauses = Xs::new(seed ^ 1);
            let mut full_hits = 0;
            for seq in 0..ITEMS {
                let mut item = Item {
                    seq,
                    tag: tag_of(seed, seq),
                    drops: Arc::clone(&producer_drops),
                };
                while let Err(back) = push(item) {
                    item = back;
                    full_hits += 1;
                    full_seen.store(true, Ordering::Release);
                    std::thread::yield_now();
                }
                pauses.pause();
            }
            full_hits
        });
        let consumer = s.spawn(move || {
            assert!(pop().is_none(), "empty before the first push");
            ready.store(true, Ordering::Release);
            // hold off until the producer has filled the ring and met the full slot
            while !full_seen.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            let mut pauses = Xs::new(seed ^ 2);
            let (mut next, mut sum) = (0, 0u64);
            while next < ITEMS {
                match pop() {
                    Some(it) => {
                        assert_eq!(it.seq, next, "order");
                        assert_eq!(it.tag, tag_of(seed, next), "payload of item {next}");
                        sum = sum.wrapping_add(it.tag);
                        next += 1;
                        pauses.pause();
                    }
                    None => std::thread::yield_now(),
                }
            }
            sum
        });
        (producer.join().unwrap(), consumer.join().unwrap())
    });
    Report {
        sum,
        full_hits,
        drops: drops.load(Ordering::Relaxed),
    }
}

fn expected_sum(seed: u64) -> u64 {
    (0..ITEMS).fold(0, |acc, k| acc.wrapping_add(tag_of(seed, k)))
}

fn check(report: &Report, seed: u64, what: &str) {
    assert_eq!(
        report.sum,
        expected_sum(seed),
        "{what}: every payload arrived"
    );
    assert!(
        report.full_hits >= 1,
        "{what}: the producer met a full ring"
    );
    assert_eq!(
        report.drops, ITEMS as usize,
        "{what}: each item dropped exactly once"
    );
}

fn owned<const N: usize>() -> Report {
    let (mut tx, mut rx) = Ring::<Item, N>::channel();
    stress(SEED, move |i| tx.push(i), move || rx.pop())
}

#[test]
fn the_owned_ends_deliver_the_whole_sequence_in_order_through_rings_of_every_small_size() {
    check(&owned::<1>(), SEED, "one slot");
    check(&owned::<2>(), SEED, "two slots");
    check(&owned::<3>(), SEED, "three slots (not a power of two)");
    check(&owned::<8>(), SEED, "eight slots");
}

#[test]
fn the_borrowed_ends_deliver_the_whole_sequence_in_order_under_a_scope() {
    let mut ring = Ring::<Item, 4>::new();
    let (mut p, mut c) = ring.split();
    let report = stress(SEED, move |i| p.push(i), move || c.pop());
    check(&report, SEED, "borrowed ends");
    assert!(ring.is_empty(), "the consumer took everything");
}

#[test]
fn a_different_seed_changes_the_payloads_and_the_checksum_follows() {
    let other = SEED ^ 0xffff;
    assert_ne!(expected_sum(SEED), expected_sum(other));
    let (mut tx, mut rx) = Ring::<Item, 2>::channel();
    check(
        &stress(other, move |i| tx.push(i), move || rx.pop()),
        other,
        "other seed",
    );
}
