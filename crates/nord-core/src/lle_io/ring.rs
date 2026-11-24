//! Wait-free single-producer / single-consumer ring buffer, pre-allocated at init.
//!
//! Slots are `[AtomicU64; 2]` (16 bytes) so the implementation needs no `unsafe`. Items implement
//! [`Wire`] to pack into those 16 bytes. `push` and `pop` do a bounded number of atomic
//! operations: no allocation, no lock, no syscall, no loop. The split into [`Producer`] and
//! [`Consumer`] (both `&mut self`, neither `Clone`) makes the single-producer / single-consumer
//! contract a compile-time property.
//!
//! **Overflow policy:** a full ring rejects the *newest* item and counts it ([`Producer::dropped`]).
//! A rejected `NoteOff` or pedal release would leave a note hanging, so the consumer can detect new
//! drops with [`Consumer::take_new_drops`] and resynchronise (see `LleInput::drain`, which injects
//! all-notes-off and pedal-off).

use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// Fixed 16-byte wire form of a ring item.
pub trait Wire: Copy {
    /// Packs `self` into 16 bytes.
    fn to_wire(self) -> [u8; 16];
    /// Unpacks 16 bytes; must accept all-zero bytes (an unwritten slot).
    fn from_wire(bytes: [u8; 16]) -> Self;
}

struct Shared {
    slots: Box<[[AtomicU64; 2]]>,
    mask: usize,
    /// Next slot to read; written by the consumer only.
    head: AtomicUsize,
    /// Next slot to write; written by the producer only.
    tail: AtomicUsize,
    dropped: AtomicU32,
}

/// Producer end (host thread: MIDI callback, GUI).
pub struct Producer<T: Wire> {
    shared: Arc<Shared>,
    _t: PhantomData<fn(T)>,
}

/// Consumer end (engine thread).
pub struct Consumer<T: Wire> {
    shared: Arc<Shared>,
    /// `dropped` value at the last [`Consumer::take_new_drops`]; consumer-local, no atomics.
    seen_dropped: u32,
    _t: PhantomData<fn() -> T>,
}

/// Creates a ring holding at least `capacity` items (rounded up to a power of two, minimum 2).
/// Allocates here, once; never afterwards.
pub fn channel<T: Wire>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    let cap = capacity.max(2).next_power_of_two();
    let shared = Arc::new(Shared {
        slots: (0..cap)
            .map(|_| [AtomicU64::new(0), AtomicU64::new(0)])
            .collect(),
        mask: cap - 1,
        head: AtomicUsize::new(0),
        tail: AtomicUsize::new(0),
        dropped: AtomicU32::new(0),
    });
    (
        Producer {
            shared: Arc::clone(&shared),
            _t: PhantomData,
        },
        Consumer {
            shared,
            seen_dropped: 0,
            _t: PhantomData,
        },
    )
}

fn split(bytes: [u8; 16]) -> [u64; 2] {
    let mut lo = [0u8; 8];
    let mut hi = [0u8; 8];
    lo.copy_from_slice(&bytes[..8]);
    hi.copy_from_slice(&bytes[8..]);
    [u64::from_le_bytes(lo), u64::from_le_bytes(hi)]
}

fn join(words: [u64; 2]) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&words[0].to_le_bytes());
    b[8..].copy_from_slice(&words[1].to_le_bytes());
    b
}

impl<T: Wire> Producer<T> {
    /// Enqueues `item`; `false` (and a drop is counted) when the ring is full.
    pub fn push(&mut self, item: T) -> bool {
        let s = &*self.shared;
        let tail = s.tail.load(Ordering::Relaxed);
        let head = s.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) > s.mask {
            s.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let w = split(item.to_wire());
        let slot = &s.slots[tail & s.mask];
        slot[0].store(w[0], Ordering::Relaxed);
        slot[1].store(w[1], Ordering::Relaxed);
        s.tail.store(tail.wrapping_add(1), Ordering::Release);
        true
    }

    /// Items rejected because the ring was full since creation.
    pub fn dropped(&self) -> u32 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Slot count (power of two).
    pub fn capacity(&self) -> usize {
        self.shared.mask + 1
    }
}

impl<T: Wire> Consumer<T> {
    /// Dequeues the oldest item, or `None` when empty.
    pub fn pop(&mut self) -> Option<T> {
        let s = &*self.shared;
        let head = s.head.load(Ordering::Relaxed);
        if head == s.tail.load(Ordering::Acquire) {
            return None;
        }
        let slot = &s.slots[head & s.mask];
        let w = [
            slot[0].load(Ordering::Relaxed),
            slot[1].load(Ordering::Relaxed),
        ];
        s.head.store(head.wrapping_add(1), Ordering::Release);
        Some(T::from_wire(join(w)))
    }

    /// Items currently queued (a lower bound while the producer runs).
    pub fn len(&self) -> usize {
        let s = &*self.shared;
        s.tail
            .load(Ordering::Acquire)
            .wrapping_sub(s.head.load(Ordering::Relaxed))
    }

    /// `true` when nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Items rejected because the ring was full since creation.
    pub fn dropped(&self) -> u32 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Drops since the previous call (wrapping); `0` when nothing was lost. Wait-free.
    pub fn take_new_drops(&mut self) -> u32 {
        let now = self.shared.dropped.load(Ordering::Relaxed);
        let new = now.wrapping_sub(self.seen_dropped);
        self.seen_dropped = now;
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Item(u64);

    impl Wire for Item {
        fn to_wire(self) -> [u8; 16] {
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&self.0.to_le_bytes());
            b[8..].copy_from_slice(&(!self.0).to_le_bytes());
            b
        }
        fn from_wire(b: [u8; 16]) -> Self {
            let mut lo = [0u8; 8];
            lo.copy_from_slice(&b[..8]);
            let mut hi = [0u8; 8];
            hi.copy_from_slice(&b[8..]);
            let (lo, hi) = (u64::from_le_bytes(lo), u64::from_le_bytes(hi));
            assert_eq!(lo, !hi, "torn slot");
            Item(lo)
        }
    }

    #[test]
    fn fifo_order_and_empty() {
        let (mut p, mut c) = channel::<Item>(4);
        assert!(c.is_empty());
        assert_eq!(c.pop(), None);
        for i in 0..3 {
            assert!(p.push(Item(i)));
        }
        assert_eq!(c.len(), 3);
        assert_eq!(
            (c.pop(), c.pop(), c.pop(), c.pop()),
            (Some(Item(0)), Some(Item(1)), Some(Item(2)), None)
        );
    }

    #[test]
    fn full_ring_rejects_and_counts_drops() {
        let (mut p, mut c) = channel::<Item>(3); // rounds to 4
        assert_eq!(p.capacity(), 4);
        for i in 0..4 {
            assert!(p.push(Item(i)));
        }
        assert!(!p.push(Item(99)));
        assert_eq!((p.dropped(), c.dropped()), (1, 1));
        assert_eq!(c.pop(), Some(Item(0)));
        assert!(p.push(Item(4)), "a freed slot is reusable");
    }

    #[test]
    fn take_new_drops_reports_each_loss_once() {
        let (mut p, mut c) = channel::<Item>(2);
        assert_eq!(c.take_new_drops(), 0);
        assert!(p.push(Item(1)) && p.push(Item(2)));
        assert!(!p.push(Item(3)) && !p.push(Item(4)));
        assert_eq!(c.take_new_drops(), 2);
        assert_eq!(c.take_new_drops(), 0);
        assert!(c.pop().is_some() && c.pop().is_some());
        assert!(p.push(Item(5)) && p.push(Item(6)) && !p.push(Item(7)));
        assert_eq!(c.take_new_drops(), 1);
    }

    #[test]
    fn wraps_around_many_times() {
        let (mut p, mut c) = channel::<Item>(2);
        for i in 0..1000u64 {
            assert!(p.push(Item(i)));
            assert_eq!(c.pop(), Some(Item(i)));
        }
    }

    /// Producer thread pushes 0..N in order (spinning when full); the consumer must see exactly
    /// 0..N, in order, with nothing lost, duplicated or torn.
    #[test]
    fn concurrent_producer_consumer_preserves_order() {
        const N: u64 = 200_000;
        let (mut p, mut c) = channel::<Item>(64);
        let prod = std::thread::spawn(move || {
            for i in 0..N {
                while !p.push(Item(i)) {
                    std::hint::spin_loop();
                }
            }
        });
        let mut next = 0u64;
        while next < N {
            if let Some(Item(v)) = c.pop() {
                assert_eq!(v, next, "out of order");
                next += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        prod.join().unwrap();
        assert_eq!(c.pop(), None);
    }
}
