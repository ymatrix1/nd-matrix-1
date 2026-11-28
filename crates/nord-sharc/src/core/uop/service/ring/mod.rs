//! A fixed-capacity single-producer single-consumer ring: no lock, no allocation after `new`.
//!
//! OWNER: W1-SVC. The translation service passes `TransReq` to the worker and `TransDone` back
//! through two of these, so the audio side never blocks and never allocates. Soundness comes from
//! the type: there is never a second producer or consumer.
//!
//! * [`Ring::split`] borrows the ring mutably and hands out one [`Producer`] and one
//!   [`Consumer`] (scoped threads, tests).
//! * [`Ring::channel`] returns one owned [`Sender`] and one owned [`Receiver`] that share the
//!   ring through an `Arc` (a detached worker thread, which needs `'static` ends).
//!
//! None of the four ends is `Clone` and every mutating call takes `&mut self`. Memory order: the
//! producer writes a slot, then publishes it with a release store of `tail`; the consumer
//! acquires `tail`, reads the slot, then hands it back with a release store of `head`; the
//! producer acquires `head` before it reuses the slot.
//!
//! Provenance: the layout is the textbook bounded SPSC queue (Lamport 1977, with acquire and
//! release in place of sequential consistency). Nothing here is measured, so every capacity
//! chosen by a caller is `ASSUMED` at the call site. The seeded two-thread stress is in
//! `stress.rs`.
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A ring of `N` slots. Positions are monotonic counters (`tail - head` items are queued); they
/// never wrap in practice (2^64 operations), ASSUMED.
pub struct Ring<T, const N: usize> {
    buf: Box<[UnsafeCell<MaybeUninit<T>>]>,
    /// Next position to pop; written by the consumer only.
    head: AtomicUsize,
    /// Next position to push; written by the producer only.
    tail: AtomicUsize,
}

// SAFETY: a slot is written by the producer before `tail` is released and read by the consumer
// after it acquires `tail`, then handed back through `head`; `split` and `channel` ensure one
// of each.
unsafe impl<T: Send, const N: usize> Sync for Ring<T, N> {}

impl<T, const N: usize> Default for Ring<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Ring<T, N> {
    /// An empty ring. Panics when `N` is zero. This is the only allocation of the ring's life.
    pub fn new() -> Self {
        assert!(N > 0, "a ring needs at least one slot");
        Ring {
            buf: (0..N)
                .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
                .collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Slots in the ring.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Items queued now (exact when called from the producer or the consumer thread).
    pub fn len(&self) -> usize {
        // `tail` first: the consumer only moves `head` up to the `tail` it saw, so a `head`
        // read after it can never be ahead of it.
        let tail = self.tail.load(Ordering::Acquire);
        tail.wrapping_sub(self.head.load(Ordering::Acquire))
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether every slot is taken.
    pub fn is_full(&self) -> bool {
        self.len() == N
    }

    /// The two borrowed ends of the ring.
    pub fn split(&mut self) -> (Producer<'_, T, N>, Consumer<'_, T, N>) {
        (Producer { ring: self }, Consumer { ring: self })
    }

    /// A fresh ring and its two owned ends, for a thread that needs `'static` handles. The ring
    /// is freed (and any item still queued dropped) when both ends are gone.
    pub fn channel() -> (Sender<T, N>, Receiver<T, N>) {
        let ring = Arc::new(Ring::new());
        (
            Sender {
                ring: Arc::clone(&ring),
            },
            Receiver { ring },
        )
    }

    fn push(&self, v: T) -> Result<(), T> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail.wrapping_sub(self.head.load(Ordering::Acquire)) == N {
            return Err(v);
        }
        // SAFETY: the slot at `tail` is free (the ring is not full) and only the producer writes.
        unsafe { (*self.buf[tail % N].get()).write(v) };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    fn pop(&self) -> Option<T> {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: the producer released this slot with `tail`; only the consumer reads it.
        let v = unsafe { (*self.buf[head % N].get()).assume_init_read() };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(v)
    }
}

impl<T, const N: usize> Drop for Ring<T, N> {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

/// The borrowed producing end: pushes without blocking.
pub struct Producer<'a, T, const N: usize> {
    ring: &'a Ring<T, N>,
}

impl<T, const N: usize> Producer<'_, T, N> {
    /// Queues `v`, or gives it back when the ring is full.
    pub fn push(&mut self, v: T) -> Result<(), T> {
        self.ring.push(v)
    }

    /// Whether every slot is taken (the consumer may free one at any time).
    pub fn is_full(&self) -> bool {
        self.ring.is_full()
    }
}

/// The borrowed consuming end: pops without blocking.
pub struct Consumer<'a, T, const N: usize> {
    ring: &'a Ring<T, N>,
}

impl<T, const N: usize> Consumer<'_, T, N> {
    /// Takes the oldest item, if any.
    pub fn pop(&mut self) -> Option<T> {
        self.ring.pop()
    }

    /// Items queued now (the producer may add more at any time).
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Whether nothing is queued now.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

/// The owned producing end of [`Ring::channel`].
pub struct Sender<T, const N: usize> {
    ring: Arc<Ring<T, N>>,
}

impl<T, const N: usize> Sender<T, N> {
    /// Queues `v`, or gives it back when the ring is full.
    pub fn push(&mut self, v: T) -> Result<(), T> {
        self.ring.push(v)
    }

    /// Whether every slot is taken (the receiver may free one at any time).
    pub fn is_full(&self) -> bool {
        self.ring.is_full()
    }

    /// Items queued now (the receiver may take some at any time).
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Whether nothing is queued now.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

/// The owned consuming end of [`Ring::channel`].
pub struct Receiver<T, const N: usize> {
    ring: Arc<Ring<T, N>>,
}

impl<T, const N: usize> Receiver<T, N> {
    /// Takes the oldest item, if any.
    pub fn pop(&mut self) -> Option<T> {
        self.ring.pop()
    }

    /// Items queued now (the sender may add more at any time).
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Whether nothing is queued now.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

#[cfg(test)]
mod stress;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::count_alloc::allocs_in;
    use std::rc::Rc;

    #[test]
    fn items_come_out_in_order_across_the_wrap_and_a_full_ring_gives_the_item_back() {
        let mut r: Ring<u32, 3> = Ring::new();
        assert_eq!((r.capacity(), r.is_empty()), (3, true));
        let (mut p, mut c) = r.split();
        assert_eq!(c.pop(), None);
        for k in 0..3 {
            assert_eq!(p.push(k), Ok(()));
        }
        assert!(p.is_full());
        assert_eq!(p.push(99), Err(99), "full");
        let mut seen = Vec::new();
        for k in 3..10 {
            seen.push(c.pop().unwrap());
            assert_eq!(p.push(k), Ok(()));
        }
        while let Some(v) = c.pop() {
            seen.push(v);
        }
        assert_eq!(seen, (0..10).collect::<Vec<_>>());
        assert!(r.is_empty());
    }

    #[test]
    fn a_one_slot_ring_alternates_between_empty_and_full() {
        let mut r: Ring<u8, 1> = Ring::new();
        let (mut p, mut c) = r.split();
        for k in 0..5u8 {
            assert!(c.is_empty() && c.pop().is_none(), "empty before {k}");
            assert_eq!(p.push(k), Ok(()));
            assert!(p.is_full() && c.len() == 1);
            assert_eq!(p.push(k + 100), Err(k + 100), "full after {k}");
            assert_eq!(c.pop(), Some(k));
        }
    }

    #[test]
    #[should_panic(expected = "at least one slot")]
    fn a_ring_without_slots_is_refused() {
        let _ = Ring::<u8, 0>::new();
    }

    #[test]
    fn queued_items_are_dropped_with_the_ring() {
        let token = Rc::new(());
        {
            let mut r: Ring<Rc<()>, 4> = Ring::new();
            let (mut p, _c) = r.split();
            p.push(token.clone()).unwrap();
            p.push(token.clone()).unwrap();
            assert_eq!(Rc::strong_count(&token), 3);
        }
        assert_eq!(Rc::strong_count(&token), 1);
    }

    /// Counts its own drops, so a leak or a double drop shows as a wrong total.
    struct Counted(Arc<AtomicUsize>);
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn items_queued_across_the_wrap_are_dropped_exactly_once() {
        let drops = Arc::new(AtomicUsize::new(0));
        {
            let (mut tx, mut rx) = Ring::<Counted, 4>::channel();
            for _ in 0..3 {
                assert!(tx.push(Counted(drops.clone())).is_ok());
            }
            drop((rx.pop().unwrap(), rx.pop().unwrap()));
            assert_eq!(
                drops.load(Ordering::Relaxed),
                2,
                "popped items drop at once"
            );
            // the queue now wraps the end of the buffer: slots 2, 3, 0, 1 hold four items
            for _ in 0..3 {
                assert!(tx.push(Counted(drops.clone())).is_ok());
            }
            assert!(tx.is_full());
            let refused = tx.push(Counted(drops.clone())).unwrap_err();
            drop(refused);
            assert_eq!(
                drops.load(Ordering::Relaxed),
                3,
                "a refused item is handed back"
            );
            drop(tx);
            assert_eq!(
                drops.load(Ordering::Relaxed),
                3,
                "the receiver keeps the ring alive"
            );
        }
        assert_eq!(
            drops.load(Ordering::Relaxed),
            3 + 4,
            "the four queued items drop once"
        );
    }

    #[test]
    fn the_owned_ends_report_the_same_queue_as_the_ring() {
        let (mut tx, mut rx) = Ring::<u16, 2>::channel();
        assert!(tx.is_empty() && rx.is_empty() && !tx.is_full());
        tx.push(7).unwrap();
        tx.push(8).unwrap();
        assert!(tx.is_full());
        assert_eq!((tx.len(), rx.len()), (2, 2));
        assert_eq!(tx.push(9), Err(9));
        assert_eq!((rx.pop(), rx.pop(), rx.pop()), (Some(7), Some(8), None));
    }

    #[test]
    fn nothing_allocates_after_new() {
        // a payload that owns heap memory moves through the ring without a copy or a new box
        let mut held: Vec<Box<[u16]>> = (0..3u16).map(|k| vec![k; 4].into_boxed_slice()).collect();
        let mut r: Ring<Box<[u16]>, 3> = Ring::new();
        let ((), n) = allocs_in(|| {
            let (mut p, mut c) = r.split();
            for _ in 0..40 {
                while let Some(b) = held.pop() {
                    // no `unwrap` here: its error arm would format the box
                    if p.push(b).is_err() {
                        unreachable!("three items fit a ring of three");
                    }
                }
                assert!(p.is_full() && c.len() == 3);
                while let Some(b) = c.pop() {
                    held.push(b); // the capacity of `held` is still three
                }
                assert!(c.is_empty());
            }
        });
        assert_eq!(
            (n, held.len()),
            (0, 3),
            "push, pop, len and the end checks allocate nothing"
        );
        let (mut tx, mut rx) = Ring::<u64, 4>::channel();
        let ((), n) = allocs_in(|| {
            for k in 0..64u64 {
                assert!(tx.push(k).is_ok());
                assert_eq!(rx.pop(), Some(k));
            }
            assert!(tx.is_empty() && rx.is_empty() && !tx.is_full());
        });
        assert_eq!(n, 0, "the owned ends allocate nothing either");
    }
}
