//! TD-18 evidence: a program load or an FX type change on the audio thread makes no heap
//! allocation and frees nothing once the firmware Mod 1 / Mod 2 stage boxes exist.
//!
//! `standalone_engine` reserves the boxes (`Stage4Engine::reserve_audio_stages`); the counting
//! allocator (`support`) sees every allocator call of the armed "audio" region, which applies
//! `AudioCommand::LoadProgram` like `apply_audio_command` does and renders blocks.

mod support;

use nord_core::cbin::{FxDecoded, ProgramDecoded};
use nord_core::engine::Stage4Engine;
use nord_core::render_opts::FirmwareOptions;
use nord_gui::audio::{apply_realtime, garbage, standalone_engine, AudioCommand};
use support::{count_allocator_calls, Counting};

#[global_allocator]
static ALLOC: Counting = Counting;

/// A program whose seven FX chains all run Mod 2 of raw type `ty` (and Mod 1 of type `m1`).
fn program(ty: u8, m1: u8) -> ProgramDecoded {
    let mut p = ProgramDecoded::default();
    let set = |fx: &mut FxDecoded| {
        fx.mod2_enabled = true;
        fx.mod2_type_raw = ty;
        fx.mod2_rate = 0.5;
        fx.mod2_amount = 0.6;
        fx.mod1_enabled = true;
        fx.mod1_type_raw = m1;
        fx.mod1_rate = 0.4;
        fx.mod1_amount = 0.5;
    };
    set(&mut p.organ.fx);
    for fx in [&mut p.piano.fx_a, &mut p.piano.fx_b] {
        set(fx);
    }
    for fx in [&mut p.synth.fx_a, &mut p.synth.fx_b, &mut p.synth.fx_c] {
        set(fx);
    }
    p
}

/// The audio callback's work for one program change: apply it, then render blocks.
fn load_and_render(engine: &mut Stage4Engine, ty: u8, m1: u8) -> usize {
    let prog = Box::new(program(ty, m1));
    let mut vol = 0.85_f32;
    // The callback's bin: the spent program box goes to a reaper (here: this thread, later).
    let (bin, reaper) = garbage::channel();
    let calls = count_allocator_calls(|| {
        apply_realtime(AudioCommand::LoadProgram(prog), engine, &mut vol, &bin);
        let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
        for _ in 0..3 {
            engine.process_block(&mut l, &mut r);
        }
    });
    assert_eq!(
        reaper.try_iter().count(),
        1,
        "the program box reached the reaper"
    );
    calls
}

/// Mod 2 types 0..=11 (chorus 4/5, vibe 6/7, law 0..=3 and 8..=11) and Mod 1 types 4..=11.
const CYCLE: [(u8, u8); 10] = [
    (4, 4),
    (6, 6),
    (0, 10),
    (9, 5),
    (5, 11),
    (2, 7),
    (7, 8),
    (11, 9),
    (3, 4),
    (4, 6),
];

#[test]
fn a_reserved_engine_changes_programs_without_allocating() {
    let mut e = standalone_engine(48_000.0);
    FirmwareOptions::default().apply_exact(&mut e);
    for (ty, m1) in CYCLE {
        let calls = load_and_render(&mut e, ty, m1);
        assert_eq!(
            calls, 0,
            "Mod 2 type {ty}, Mod 1 type {m1}: {calls} allocator calls"
        );
    }
}

#[test]
fn without_the_reservation_the_first_use_builds_the_boxes_on_the_audio_thread() {
    // The pre-fix behaviour (a plain engine): every stage kind is boxed lazily on the audio thread.
    let mut e = Stage4Engine::new(48_000.0);
    FirmwareOptions::default().apply_exact(&mut e);
    let calls: usize = CYCLE
        .iter()
        .map(|&(ty, m1)| load_and_render(&mut e, ty, m1))
        .sum();
    eprintln!("lazy stage builds over the cycle: {calls} allocator calls");
    assert!(
        calls > 20,
        "only {calls} allocator calls: the counter would miss a regression"
    );
}
