//! The LLE live-I/O bridge in a headless host (bean `ns4-lle6`): with the bridge off the audio is
//! byte-identical to the path without it, and with it on the process callback allocates nothing,
//! keeps sample offsets and counts overflow instead of blocking. Own test binary: it installs a
//! counting `#[global_allocator]` (armed per thread, only around `process()`).

use nih_plug::prelude::*;
use nord_core::engine_select::LleAvailability;
use nord_core::lle_io::TelemetrySnapshot;
use nord_plugin::lle_bridge::fake::FakeBackend;
use nord_plugin::lle_bridge::{drain_stamped, BridgeState, LleBridge};
use nord_plugin::NdMatrix1Plugin;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::VecDeque;
use std::mem::MaybeUninit;

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

#[global_allocator]
static ALLOC: Counting = Counting;

fn count_allocator_calls(f: impl FnOnce()) -> usize {
    EVENTS.with(|n| n.set(0));
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    EVENTS.with(Cell::get)
}

struct Init;
impl InitContext<NdMatrix1Plugin> for Init {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }
    fn execute(&self, _task: ()) {}
    fn set_latency_samples(&self, _samples: u32) {}
    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

struct Ctx {
    transport: Transport,
    events: VecDeque<NoteEvent<()>>,
}

impl ProcessContext<NdMatrix1Plugin> for Ctx {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }
    fn execute_background(&self, _task: ()) {}
    fn execute_gui(&self, _task: ()) {}
    fn transport(&self) -> &Transport {
        &self.transport
    }
    fn next_event(&mut self) -> Option<NoteEvent<()>> {
        self.events.pop_front()
    }
    fn send_event(&mut self, _event: NoteEvent<()>) {}
    fn set_latency_samples(&self, _samples: u32) {}
    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

const BLOCK: usize = 128;
const BLOCKS: usize = 48;

fn new_ctx() -> Ctx {
    // SAFETY: `Transport` is plain data; all-zero is a valid value.
    let mut transport: Transport = unsafe { MaybeUninit::zeroed().assume_init() };
    transport.sample_rate = 48_000.0;
    transport.tempo = Some(120.0);
    Ctx {
        transport,
        events: VecDeque::with_capacity(256),
    }
}

fn activate() -> NdMatrix1Plugin {
    let mut plugin = NdMatrix1Plugin::default();
    plugin.library_source = nord_plugin::auto_library::LibrarySource::None;
    assert!(plugin.initialize(
        &NdMatrix1Plugin::AUDIO_IO_LAYOUTS[0],
        &BufferConfig {
            sample_rate: 48_000.0,
            min_buffer_size: None,
            max_buffer_size: BLOCK as u32,
            process_mode: ProcessMode::Offline,
        },
        &mut Init,
    ));
    plugin
}

/// The fixed MIDI script for block `n`: events at known sample offsets.
fn script(n: usize, ctx: &mut Ctx) {
    let note = |timing, on: bool, note| {
        if on {
            NoteEvent::NoteOn {
                timing,
                voice_id: None,
                channel: 0,
                note,
                velocity: 0.7,
            }
        } else {
            NoteEvent::NoteOff {
                timing,
                voice_id: None,
                channel: 0,
                note,
                velocity: 0.0,
            }
        }
    };
    match n {
        0 => {
            ctx.events.push_back(note(0, true, 60));
            ctx.events.push_back(note(37, true, 64));
            ctx.events.push_back(note(90, true, 67));
        }
        4 => {
            ctx.events.push_back(NoteEvent::MidiCC {
                timing: 5,
                channel: 0,
                cc: 64,
                value: 1.0,
            });
            ctx.events.push_back(NoteEvent::MidiPitchBend {
                timing: 20,
                channel: 0,
                value: 0.75,
            });
            ctx.events.push_back(NoteEvent::MidiChannelPressure {
                timing: 20,
                channel: 0,
                pressure: 0.6,
            });
        }
        8 => {
            ctx.events.push_back(note(1, false, 60));
            ctx.events.push_back(NoteEvent::MidiProgramChange {
                timing: 50,
                channel: 0,
                program: 3,
            });
            ctx.events.push_back(NoteEvent::MidiCC {
                timing: 127,
                channel: 0,
                cc: 1,
                value: 0.5,
            });
        }
        12 => ctx.events.push_back(NoteEvent::MidiCC {
            timing: 0,
            channel: 0,
            cc: 64,
            value: 0.0,
        }),
        _ => {}
    }
}

/// FNV-1a over the bit patterns of every output sample.
fn render_hash(
    plugin: &mut NdMatrix1Plugin,
    mut after_block: impl FnMut(usize, &mut NdMatrix1Plugin),
) -> u64 {
    let mut ctx = new_ctx();
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for n in 0..BLOCKS {
        script(n, &mut ctx);
        let slices: Vec<&mut [f32]> = vec![&mut l[..], &mut r[..]];
        let mut buffer = Buffer::default();
        // SAFETY: the slices outlive `buffer` and are exactly BLOCK frames long.
        unsafe {
            buffer.set_slices(BLOCK, |output| {
                *output = slices;
            });
        }
        let mut aux = AuxiliaryBuffers {
            inputs: &mut [],
            outputs: &mut [],
        };
        assert_eq!(
            plugin.process(&mut buffer, &mut aux, &mut ctx),
            ProcessStatus::Normal
        );
        for s in buffer.as_slice().iter().flat_map(|c| c.iter()) {
            for b in s.to_bits().to_le_bytes() {
                hash = (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        after_block(n, plugin);
    }
    hash
}

/// Hash of the script through the plugin without any bridge activity.
/// VERIFIED: recorded by running this test body on `main` before the bridge existed
/// (same `render_hash` body in a scratch test binary, printed `0x7fbcedeaa5919a1c`), 2 channels x
/// 48 blocks x 128 frames at 48 kHz, default scene, OmniSplit.
/// Float output differs between platforms (libm, FMA), so the recorded value is only checked on
/// Linux x86_64 (macOS CI printed another hash for the same script); every platform compares the
/// bridge variants against a baseline rendered in the same process instead.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const GOLDEN_HASH: u64 = 0x7fbc_edea_a591_9a1c;

/// Hash of the script with the bridge off, rendered here (the path without the bridge).
fn baseline_hash() -> u64 {
    let mut plugin = activate();
    assert_eq!(plugin.lle.state(), BridgeState::Off);
    render_hash(&mut plugin, |_, _| {})
}

#[test]
fn bridge_off_is_byte_identical_to_the_path_without_it() {
    let first = baseline_hash();
    assert_eq!(first, baseline_hash(), "the render is deterministic");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    assert_eq!(first, GOLDEN_HASH, "differs from the hash recorded on main");
}

#[test]
fn requested_without_an_engine_keeps_the_normal_engine_and_the_same_audio() {
    let mut plugin = activate();
    plugin.lle = LleBridge::from_settings_with(Some("lle"), None, || {
        LleAvailability::Unavailable("none".into())
    });
    assert_eq!(
        plugin.lle.status_text(),
        Some("LLE engine not available in this build")
    );
    assert_eq!(render_hash(&mut plugin, |_, _| {}), baseline_hash());
    assert_eq!(plugin.lle.overflow_count(), 0);
}

#[test]
fn an_active_bridge_does_not_change_the_audio_and_keeps_sample_offsets() {
    let mut plugin = activate();
    plugin.lle = LleBridge::from_settings_with(Some("lle"), None, || LleAvailability::Available);
    plugin.lle.bind_params(&plugin.params);
    let mut ends = plugin.lle.take_engine_ends().expect("engine ends");
    let mut fake = FakeBackend::new();
    let hash = render_hash(&mut plugin, |_, p| {
        drain_stamped(&mut fake, &mut ends.events);
        assert_eq!(p.lle.overflow_count(), 0);
    });
    assert_eq!(hash, baseline_hash(), "forwarding must not touch the audio");
    // Block-relative offsets and order, exactly as the script sent them.
    let offsets: Vec<u32> = fake.log().iter().map(|e| e.0).collect();
    assert_eq!(offsets, [0, 37, 90, 5, 20, 20, 1, 50, 127, 0]);
    use nord_core::lle_io::{LleEvent, Pedal};
    let kinds: Vec<&LleEvent> = fake.log().iter().map(|e| &e.1).collect();
    assert!(matches!(kinds[0], LleEvent::NoteOn { note: 60, .. }));
    assert!(matches!(
        kinds[3],
        LleEvent::Pedal {
            pedal: Pedal::Sustain,
            value: 127,
            ..
        }
    ));
    assert!(matches!(kinds[4], LleEvent::PitchBend { .. }));
    assert!(matches!(kinds[5], LleEvent::ChannelAftertouch { .. }));
    assert!(matches!(
        kinds[7],
        LleEvent::ProgramChange { program: 3, .. }
    ));
    assert!(matches!(
        kinds[9],
        LleEvent::Pedal {
            pedal: Pedal::Sustain,
            value: 0,
            ..
        }
    ));
}

#[test]
fn a_flooded_ring_is_counted_and_process_still_returns() {
    let mut plugin = activate();
    plugin.lle = LleBridge::active(4);
    let mut ctx = new_ctx();
    for i in 0..64u8 {
        ctx.events.push_back(NoteEvent::NoteOn {
            timing: u32::from(i),
            voice_id: None,
            channel: 0,
            note: 40 + i % 40,
            velocity: 0.5,
        });
    }
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let slices: Vec<&mut [f32]> = vec![&mut l[..], &mut r[..]];
    let mut buffer = Buffer::default();
    // SAFETY: the slices outlive `buffer` and are BLOCK frames long.
    unsafe {
        buffer.set_slices(BLOCK, |output| {
            *output = slices;
        });
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    plugin.process(&mut buffer, &mut aux, &mut ctx);
    assert_eq!(plugin.lle.overflow_count(), 60, "4 slots kept, 60 rejected");
}

#[test]
fn process_with_an_active_bridge_and_backend_allocates_nothing() {
    let mut plugin = activate();
    plugin.lle = LleBridge::from_settings_with(Some("lle"), None, || LleAvailability::Available);
    plugin.lle.bind_params(&plugin.params);
    assert!(plugin.lle.attach_backend(Box::new(FakeBackend::new())));
    let mut ctx = new_ctx();
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let slices: Vec<&mut [f32]> = vec![&mut l[..], &mut r[..]];
    let mut buffer = Buffer::default();
    // SAFETY: the slices outlive `buffer` and are BLOCK frames long.
    unsafe {
        buffer.set_slices(BLOCK, |output| {
            *output = slices;
        });
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    // Warm-up (unarmed): lazy tables may allocate once.
    for n in 0..8 {
        script(n, &mut ctx);
        plugin.process(&mut buffer, &mut aux, &mut ctx);
    }
    let mut snap = TelemetrySnapshot::default();
    let calls = count_allocator_calls(|| {
        for block in 0..300usize {
            let n = 21 + (block % 80) as u8;
            ctx.events.push_back(NoteEvent::NoteOn {
                timing: 3,
                voice_id: None,
                channel: 0,
                note: n,
                velocity: 0.6,
            });
            ctx.events.push_back(NoteEvent::MidiCC {
                timing: 9,
                channel: 0,
                cc: 64,
                value: 1.0,
            });
            ctx.events.push_back(NoteEvent::MidiPitchBend {
                timing: 11,
                channel: 0,
                value: 0.5,
            });
            ctx.events.push_back(NoteEvent::NoteOff {
                timing: 64,
                voice_id: None,
                channel: 0,
                note: n,
                velocity: 0.0,
            });
            plugin.process(&mut buffer, &mut aux, &mut ctx);
            let _ = plugin.lle.poll_telemetry(&mut snap);
        }
    });
    assert_eq!(
        calls, 0,
        "process() with the bridge made {calls} allocator calls"
    );
    assert!(snap.frame > 0, "telemetry reached the editor side");
}
