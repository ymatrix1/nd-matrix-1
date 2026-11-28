//! The one counting `#[global_allocator]` of the lib test binary, shared by every "no allocation"
//! obligation of the uop engine (the ring and the worker, `uop_try` in a steady pass, the native
//! install path).
//!
//! OWNER: W0 (skeleton, this amendment). A binary may have only one global allocator, so a
//! package must NOT define its own: two packages doing it would fail to compile when merged. The
//! counter is per thread, so tests running in parallel do not see each other's allocations.
//! Use [`allocs_in`]; it counts `alloc`, `alloc_zeroed` and `realloc` calls (a free is not an
//! allocation).
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    // const-initialised and without a destructor: reading it never allocates
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

fn bump() {
    let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
}

struct Counting;

// SAFETY: every method forwards to the system allocator with the same arguments; the only extra
// work is a thread-local counter increment that cannot allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` and returns its result with the number of allocations the current thread made in it.
pub(crate) fn allocs_in<R>(f: impl FnOnce() -> R) -> (R, u64) {
    let before = ALLOCS.with(Cell::get);
    let r = f();
    (r, ALLOCS.with(Cell::get) - before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_allocations_of_the_measuring_thread_are_counted() {
        let (v, none) = allocs_in(|| std::hint::black_box(2 + 2));
        assert_eq!((v, none), (4, 0), "arithmetic does not allocate");
        let (v, one) = allocs_in(|| Vec::<u8>::with_capacity(64));
        assert_eq!(one, 1);
        drop(v);
        let (_, grow) = allocs_in(|| {
            let mut v = Vec::<u8>::new();
            v.extend_from_slice(&[0; 8]);
            v.extend_from_slice(&[0; 4096]); // a reallocation
            std::hint::black_box(&v).len()
        });
        assert!(grow >= 2, "{grow}");
        // another thread's 1000 allocations are not ours (spawning costs only a handful)
        let (_, mine) = allocs_in(|| {
            std::thread::scope(|s| {
                let theirs = s.spawn(|| {
                    allocs_in(|| {
                        let v: Vec<Box<u32>> = (0..1000u32).map(Box::new).collect();
                        std::hint::black_box(&v).len()
                    })
                    .1
                });
                assert!(theirs.join().unwrap() >= 1000);
            })
        });
        assert!(mine < 200, "{mine}");
    }
}
