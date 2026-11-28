//! The editor to engine load path (D218): OPEN .NS4B, load a program, hear it.
//!
//! Own test binary because it installs a counting `#[global_allocator]` (same scheme as
//! `aux_outputs.rs`, armed per thread and only around `process()`), to prove that applying queued
//! loads makes no allocator call on the audio thread.

use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::EguiState;
use nord_core::cbin::ProgramDecoded;
use nord_core::midi::MidiEvent;
use nord_core::piano::NpnoSound;
use nord_core::render_opts::FirmwareOptions;
use nord_core::synth::SynthSampleSound;
use nord_gui::debug_dialog::DebugTab;
use nord_plugin::editor::draw::{draw_editor, EditorState, DEFAULT_SIZE};
use nord_plugin::editor::index::ParamIndex;
use nord_plugin::editor::top_bar::BarClick;
use nord_plugin::editor::write::{ParamWrite, ParamWriter};
use nord_plugin::loader::LoadJob;
use nord_plugin::NdMatrix1Plugin;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::mem::MaybeUninit;
use std::sync::Arc;

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
        None
    }
    fn send_event(&mut self, _event: NoteEvent<()>) {}
    fn set_latency_samples(&self, _samples: u32) {}
    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

const FRAMES: usize = 128;

fn ctx() -> Ctx {
    // SAFETY: `Transport` is plain data (numbers and options); all-zero is a valid value.
    let mut transport: Transport = unsafe { MaybeUninit::zeroed().assume_init() };
    transport.sample_rate = 48_000.0;
    transport.tempo = Some(120.0);
    Ctx { transport }
}

fn activate() -> NdMatrix1Plugin {
    let mut plugin = NdMatrix1Plugin::default();
    plugin.library_source = nord_plugin::auto_library::LibrarySource::None;
    let ok = plugin.initialize(
        &NdMatrix1Plugin::AUDIO_IO_LAYOUTS[1],
        &BufferConfig {
            sample_rate: 48_000.0,
            min_buffer_size: None,
            max_buffer_size: FRAMES as u32,
            process_mode: ProcessMode::Realtime,
        },
        &mut Init,
    );
    assert!(ok);
    plugin
}

/// One block; returns (left, right, allocator calls during `process()`).
fn process(plugin: &mut NdMatrix1Plugin) -> (Vec<f32>, Vec<f32>, usize) {
    let (mut l, mut r) = (vec![0.0f32; FRAMES], vec![0.0f32; FRAMES]);
    let mut buffer = Buffer::default();
    // SAFETY: the slices outlive `buffer` and are exactly `FRAMES` long.
    unsafe {
        buffer.set_slices(FRAMES, |output| {
            *output = vec![&mut l[..], &mut r[..]];
        });
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    let mut c = ctx();
    EVENTS.with(|n| n.set(0));
    ARMED.with(|a| a.set(true));
    plugin.process(&mut buffer, &mut aux, &mut c);
    ARMED.with(|a| a.set(false));
    let events = EVENTS.with(Cell::get);
    drop(buffer);
    (l, r, events)
}

fn play_chord(plugin: &NdMatrix1Plugin) {
    for note in [48u8, 60, 64, 67] {
        assert!(plugin.note_tx.push(MidiEvent::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        }));
    }
}

fn energy(blocks: usize, plugin: &mut NdMatrix1Plugin) -> f32 {
    (0..blocks)
        .map(|_| {
            let (l, r, _) = process(plugin);
            l.iter().chain(&r).map(|s| s.abs()).sum::<f32>()
        })
        .sum()
}

/// A program that differs from the plugin defaults in what the block sync writes: other
/// drawbars, other level.
fn program() -> ProgramDecoded {
    let mut prog = ProgramDecoded {
        name: "Load Path Test".into(),
        ..ProgramDecoded::default()
    };
    prog.organ.layer_a_level = 0.35;
    prog.organ.drawbars_a = [0, 0, 8, 8, 0, 0, 3, 0, 0];
    prog
}

fn piano() -> Arc<NpnoSound> {
    Arc::new(NpnoSound::new_synthetic(
        "Test Grand",
        60,
        &[0.1, 0.2, 0.1, 0.0],
        48_000.0,
    ))
}

#[test]
fn applying_queued_loads_makes_no_allocator_call_on_the_audio_thread() {
    let mut plugin = activate();
    // Everything is built here, on the "GUI thread".
    let jobs = vec![
        LoadJob::Program(Box::new(program())),
        LoadJob::Program(Box::new(program())), // replaces the first: its box goes to the reaper
        LoadJob::Piano {
            slot_b: false,
            sound: Some(piano()),
        },
        LoadJob::Piano {
            slot_b: false,
            sound: Some(piano()), // replaces the bank, which a reaper frees
        },
        LoadJob::Sample {
            layer: 1,
            sound: Some(Arc::new(SynthSampleSound {
                name: "S".into(),
                zones: vec![],
            })),
        },
        LoadJob::Firmware(FirmwareOptions::default()),
    ];
    for job in jobs {
        assert!(plugin.load_tx.push(job).is_ok());
    }
    play_chord(&plugin);
    let (_, _, calls) = process(&mut plugin);
    assert!(plugin.load_rx.is_empty(), "process() must drain the loads");
    assert_eq!(calls, 0, "process() with loads called the allocator");
    // And the loads took effect.
    assert_eq!(plugin.engine.organ_a.drawbars, [0, 0, 8, 8, 0, 0, 3, 0, 0]);
    assert!(plugin.engine.piano_a.sound.is_some());
    assert_eq!(plugin.engine.synth_b.loaded_sample_name(), Some("S"));
}

#[test]
fn a_loaded_program_survives_the_block_sync_and_a_host_reset() {
    let mut plugin = activate();
    assert!(plugin
        .load_tx
        .push(LoadJob::Program(Box::new(program())))
        .is_ok());
    play_chord(&plugin);
    let _ = process(&mut plugin);
    // The block sync ran after the load and left the program's drawbars and level alone.
    assert_eq!(plugin.engine.organ_a.drawbars, [0, 0, 8, 8, 0, 0, 3, 0, 0]);
    assert!((plugin.engine.layer_levels[0].base - 0.35).abs() < 1e-5);
    plugin.reset();
    assert_eq!(plugin.engine.organ_a.drawbars, [0, 0, 8, 8, 0, 0, 3, 0, 0]);
    assert!((plugin.engine.layer_levels[0].base - 0.35).abs() < 1e-5);
}

/// Without the hold the sync would put the default drawbars back within one block, and the
/// program would sound like the default plugin.
#[test]
fn a_loaded_program_sounds_like_the_program_not_like_the_defaults() {
    let mut plain = activate();
    play_chord(&plain);
    let default_energy = energy(24, &mut plain);

    let mut loaded = activate();
    assert!(loaded
        .load_tx
        .push(LoadJob::Program(Box::new(program())))
        .is_ok());
    play_chord(&loaded);
    let program_energy = energy(24, &mut loaded);

    assert!(program_energy > 0.0, "the loaded program is silent");
    assert!(
        (program_energy - default_energy).abs() > 0.05 * default_energy,
        "program {program_energy} sounds like the defaults {default_energy}"
    );
    // After the blocks the state is still the program's (drawbars, level, scene), not the host's.
    assert_eq!(loaded.engine.organ_a.drawbars, [0, 0, 8, 8, 0, 0, 3, 0, 0]);
    assert!((loaded.engine.layer_levels[0].base - 0.35).abs() < 1e-5);
}

struct Sink;
impl ParamWriter for Sink {
    fn write(&mut self, _w: &ParamWrite) -> bool {
        true
    }
}

fn frame(ctx: &egui::Context, egui_state: &EguiState, st: &mut EditorState) -> usize {
    let size = egui::Vec2::new(DEFAULT_SIZE.0 as f32, DEFAULT_SIZE.1 as f32);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..Default::default()
    };
    ctx.run(input, |ctx| draw_editor(ctx, egui_state, st, Sink))
        .shapes
        .len()
}

/// The done-criterion without a DAW: OPEN .NS4B, open a `.ns4b`, pick a program in the library
/// tab, and it sounds in `process()`.
#[test]
fn open_ns4b_then_load_a_program_and_it_sounds() {
    let mut plugin = activate();
    let index =
        Arc::new(ParamIndex::new(plugin.params.as_ref()).with_program_hold(plugin.params.clone()));
    let mut st = EditorState::with_library(
        index,
        plugin.note_tx.clone(),
        plugin.load_tx.clone(),
        plugin.telemetry.clone(),
        None,
    );
    let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
    let ctx = egui::Context::default();
    let closed = frame(&ctx, &egui_state, &mut st);

    // The OPEN .NS4B pill opens the Library tab as an embedded window.
    st.open_for(BarClick::OpenNs4b);
    assert!(st.debug_dialog.open && st.debug_dialog.tab == DebugTab::Library);
    let open = frame(&ctx, &egui_state, &mut st);
    assert!(
        open > closed,
        "the Library dialog drew nothing ({open} <= {closed})"
    );

    // A backup with one program, written where a user's file would be.
    let bytes = program().to_cbin_file(0, 0).expect("encode").to_bytes();
    let path = std::env::temp_dir().join(format!("nd-load-path-{}.ns4b", std::process::id()));
    std::fs::write(
        &path,
        nord_core::cbin::stored_archive(&[("Programs/Load Path Test.ns4p", &bytes)]).unwrap(),
    )
    .unwrap();
    st.library.open_backup_path(&path).expect("open .ns4b");
    assert_eq!(st.library.programs, ["Programs/Load Path Test.ns4p"]);
    st.library
        .load_program_entry("Programs/Load Path Test.ns4p", &mut st.panel, &st.handle);
    let _ = std::fs::remove_file(&path);

    // The next editor frame pumps the load into the queue; `process()` applies it and plays.
    let _ = frame(&ctx, &egui_state, &mut st);
    assert!(
        !plugin.load_rx.is_empty(),
        "the program never reached the load queue"
    );
    play_chord(&plugin);
    let heard = energy(24, &mut plugin);
    assert!(plugin.load_rx.is_empty());
    assert!(heard > 0.0, "the loaded program is silent");
    assert_eq!(plugin.engine.organ_a.drawbars, [0, 0, 8, 8, 0, 0, 3, 0, 0]);
    assert!(
        st.library.error_message.is_none(),
        "{:?}",
        st.library.error_message
    );
}

#[test]
fn a_load_with_no_audio_thread_running_is_reported_not_lost_silently() {
    let plugin = NdMatrix1Plugin::default();
    let index = Arc::new(ParamIndex::new(plugin.params.as_ref()));
    let mut st = EditorState::with_library(
        index,
        plugin.note_tx.clone(),
        plugin.load_tx.clone(),
        plugin.telemetry.clone(),
        None,
    );
    let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
    let ctx = egui::Context::default();
    // Nothing drains the queue (no process()): after its capacity the loads are rejected.
    for _ in 0..nord_plugin::loader::LOAD_QUEUE_CAPACITY + 1 {
        st.handle
            .send_command(nord_gui::audio::AudioCommand::LoadProgram(Box::default()));
    }
    let _ = frame(&ctx, &egui_state, &mut st);
    assert!(st
        .library
        .error_message
        .as_deref()
        .is_some_and(|m| m.contains("not processing")));
}
