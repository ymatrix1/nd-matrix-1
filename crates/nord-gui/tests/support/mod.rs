//! Counting global allocator shared by the `no_alloc_*` test binaries.
//!
//! Each test binary installs it with `#[global_allocator] static ALLOC: Counting = Counting;`. It
//! counts `alloc`, `alloc_zeroed`, `realloc` and `dealloc` only on a thread that armed a `const`
//! thread-local flag ([`count_allocator_calls`]), so test harness threads do not disturb the count.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

pub struct Counting;

fn note_event() {
    let _ = ARMED.try_with(|armed| {
        if armed.get() {
            let _ = EVENTS.try_with(|n| n.set(n.get() + 1));
        }
    });
}

// SAFETY: forwards unchanged arguments to `System`; counting touches only const thread-locals.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_event();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_event();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_event();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_event();
        unsafe { System.dealloc(ptr, layout) }
    }
}

pub fn count_allocator_calls(f: impl FnOnce()) -> usize {
    EVENTS.with(|n| n.set(0));
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    EVENTS.with(Cell::get)
}
