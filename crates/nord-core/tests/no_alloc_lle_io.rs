//! Proves the LLE live-I/O hot paths (ring push/pop, event translation, telemetry publish/read)
//! never touch the heap. Own test binary: it installs a counting `#[global_allocator]`
//! (same scheme as `no_alloc_audio_path.rs`; docs/specs/telemetry.md, "Allocation proof").

use nord_core::lle_io::{link, midi_bytes, panel_edit, LleEvent, PanelKind, TelemetrySnapshot};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

fn note_event() {
    let _ = ARMED.try_with(|armed| {
        if armed.get() {
            let _ = EVENTS.try_with(|n| n.set(n.get() + 1));
        }
    });
}

// SAFETY: forwards unchanged arguments to `System`; counting only touches const thread-locals.
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

#[global_allocator]
static ALLOC: Counting = Counting;

fn count_allocator_calls(f: impl FnOnce()) -> usize {
    EVENTS.with(|n| n.set(0));
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    EVENTS.with(Cell::get)
}

#[test]
fn the_counter_sees_allocations_and_frees() {
    let n = count_allocator_calls(|| {
        let v: Vec<u8> = Vec::with_capacity(64);
        std::hint::black_box(&v);
        drop(v);
    });
    assert_eq!(n, 2);
    assert_eq!(count_allocator_calls(|| {}), 0);
}

#[test]
fn ring_translate_and_telemetry_do_not_allocate() {
    // Setup allocates (rings, cells, buffers): unarmed.
    let (mut host, mut eng) = link(64);
    let mut snap = TelemetrySnapshot::default();
    let mut out = TelemetrySnapshot::default();
    let msgs: [&[u8]; 4] = [
        &[0x90, 60, 100],
        &[0xB0, 64, 127],
        &[0xE0, 0, 64],
        &[0xC0, 3],
    ];

    let calls = count_allocator_calls(|| {
        for round in 0..1000u32 {
            for m in msgs {
                if let Some(e) = midi_bytes(m) {
                    assert!(host.events.push(e));
                }
            }
            assert!(host
                .events
                .push(panel_edit(PanelKind::Knob, 5, 0.25).unwrap()));
            let mut n = 0;
            while let Some(e) = eng.events.pop() {
                assert_ne!(e, LleEvent::Nop);
                n += 1;
            }
            assert_eq!(n, 5);
            snap.frame = round;
            snap.set_led(7, round % 2 == 0);
            eng.telemetry.publish(&snap);
            assert!(host.telemetry.read(&mut out));
        }
    });
    assert_eq!(calls, 0, "hot path made {calls} allocator calls");
    assert_eq!(out.frame, 999);
}
