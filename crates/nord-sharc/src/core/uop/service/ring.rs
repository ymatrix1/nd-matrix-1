//! A fixed-capacity single-producer single-consumer ring: no lock, no allocation after `new`.
//!
//! OWNER: W1-SVC. The translation service passes `TransReq` to the worker and `TransDone` back
//! through two of these, so the audio side never blocks and never allocates. Soundness comes from
//! the type: [`Ring::split`] borrows the ring mutably and hands out one [`Producer`] and one
//! [`Consumer`], neither `Clone`, so there is never a second producer or consumer.
//!
//! Skeleton: complete. The package adds the counting-allocator test (no allocation after `new`)
//! and the seeded two-thread stress.
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

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
// after it acquires `tail`, then handed back through `head`; `split` ensures one of each.
unsafe impl<T: Send, const N: usize> Sync for Ring<T, N> {}

impl<T, const N: usize> Default for Ring<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Ring<T, N> {
    /// An empty ring. Panics when `N` is zero.
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
        self.tail
            .load(Ordering::Acquire)
            .wrapping_sub(self.head.load(Ordering::Acquire))
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The two ends of the ring.
    pub fn split(&mut self) -> (Producer<'_, T, N>, Consumer<'_, T, N>) {
        (Producer { ring: self }, Consumer { ring: self })
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

/// The producing end: pushes without blocking.
pub struct Producer<'a, T, const N: usize> {
    ring: &'a Ring<T, N>,
}

impl<T, const N: usize> Producer<'_, T, N> {
    /// Queues `v`, or gives it back when the ring is full.
    pub fn push(&mut self, v: T) -> Result<(), T> {
        self.ring.push(v)
    }
}

/// The consuming end: pops without blocking.
pub struct Consumer<'a, T, const N: usize> {
    ring: &'a Ring<T, N>,
}

impl<T, const N: usize> Consumer<'_, T, N> {
    /// Takes the oldest item, if any.
    pub fn pop(&mut self) -> Option<T> {
        self.ring.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn two_threads_move_a_sequence_through_a_small_ring_in_order() {
        let mut r: Ring<u64, 8> = Ring::new();
        let (mut p, mut c) = r.split();
        let n = 20_000u64;
        let sum = std::thread::scope(|s| {
            s.spawn(move || {
                let mut k = 0;
                while k < n {
                    if p.push(k).is_ok() {
                        k += 1;
                    }
                }
            });
            s.spawn(move || {
                let (mut next, mut sum) = (0, 0);
                while next < n {
                    if let Some(v) = c.pop() {
                        assert_eq!(v, next, "order");
                        (next, sum) = (next + 1, sum + v);
                    }
                }
                sum
            })
            .join()
            .unwrap()
        });
        assert_eq!(sum, n * (n - 1) / 2);
    }
}
