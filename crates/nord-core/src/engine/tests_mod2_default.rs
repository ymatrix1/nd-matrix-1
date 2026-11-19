//! The firmware Mod 2 is the default (D163): a program load puts the raw words in the chain, the
//! default chain matches the single-port firmware chain and not the legacy one, and every path
//! that restores or copies FX settings (undo, Live slots, FX clipboard) carries the words with it.

use super::*;
use crate::fx::{FirmwareFx, Mod2Type};

const CHOR: u8 = 4;
const RATE: f32 = 0.6;
const SYNTH_A: usize = 4;

/// Program with a Mod 2 of raw type `ty` in the Synth A FX.
fn program(ty: u8) -> crate::cbin::ProgramDecoded {
    let mut prog = crate::cbin::ProgramDecoded::default();
    let fx = &mut prog.synth.fx_a;
    fx.mod2_enabled = true;
    fx.mod2_type_raw = ty;
    fx.mod2_rate = RATE;
    fx.mod2_amount = 0.8;
    prog
}

fn stimulus(i: usize) -> f32 {
    ((i as f32 * 0.07).sin() + 0.3 * (i as f32 * 0.31).sin()) * 0.5
}

fn run(fx: &mut LayerFxChain, n: usize) -> Vec<[f32; 2]> {
    (0..n)
        .map(|i| fx.process_frame([stimulus(i), 0.7 * stimulus(i)]))
        .collect()
}

fn max_diff(a: &[[f32; 2]], b: &[[f32; 2]]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x[0] - y[0]).abs().max((x[1] - y[1]).abs()))
        .fold(0.0, f32::max)
}

fn loaded(ty: u8) -> Stage4Engine {
    let mut engine = Stage4Engine::new(48_000.0);
    engine.apply_program(&program(ty));
    engine
}

/// The chain a hand-wired single firmware Mod 2 port would be: the chain of a fresh engine with
/// the Mod 2 words, type and amount set and the delay and reverb mixes dry (no program load).
fn port_chain(ty: u8) -> LayerFxChain {
    let mut fx = Stage4Engine::new(48_000.0).fx_chains[SYNTH_A].clone();
    fx.mod2_type = Mod2Type::Chorus;
    fx.mod2_type_raw = Some(ty);
    fx.mod2_rate_raw = Some((RATE * 127.0).round() as u8);
    fx.mod2_amount = 0.8;
    fx.delay_mix = 0.0;
    fx.reverb_mix = 0.0;
    fx
}

#[test]
fn apply_program_on_the_default_engine_runs_the_firmware_mod2() {
    let engine = loaded(CHOR);
    let chain = &engine.fx_chains[SYNTH_A];
    assert!(chain.firmware_fx.mod2, "the firmware Mod 2 is the default");
    assert_eq!(chain.mod2_type_raw, Some(CHOR));
    let mut default = chain.clone();
    let mut port = port_chain(CHOR);
    let mut legacy = chain.clone();
    legacy.firmware_fx = FirmwareFx::LEGACY;
    let (d, p, l) = (
        run(&mut default, 8_000),
        run(&mut port, 8_000),
        run(&mut legacy, 8_000),
    );
    assert_eq!(max_diff(&d, &p), 0.0, "default chain is the firmware port");
    assert!(max_diff(&d, &l) > 0.01, "legacy chain is the engine Mod 2");
}

/// Chain after a GUI edit of the type: the engine Phaser at the loaded rate and amount.
fn edited_chain(engine: &Stage4Engine) -> LayerFxChain {
    let mut fx = engine.fx_chains[SYNTH_A].clone();
    fx.mod2_type = Mod2Type::Phaser;
    fx.clear_mod2_firmware_words();
    fx
}

#[test]
fn clearing_the_words_returns_to_the_engine_mod2() {
    let engine = loaded(CHOR);
    let mut firmware = engine.fx_chains[SYNTH_A].clone();
    firmware.mod2_type = Mod2Type::Phaser; // type edited, words left in place: ignored
    let mut edited = edited_chain(&engine);
    let mut legacy = engine.fx_chains[SYNTH_A].clone();
    legacy.mod2_type = Mod2Type::Phaser;
    legacy.firmware_fx = FirmwareFx::LEGACY;
    let (f, e, l) = (
        run(&mut firmware, 6_000),
        run(&mut edited, 6_000),
        run(&mut legacy, 6_000),
    );
    assert_eq!(max_diff(&e, &l), 0.0, "edited chain runs the engine Phaser");
    assert!(
        max_diff(&f, &e) > 0.01,
        "without the clear the edit is lost"
    );
}

#[test]
fn undo_restores_the_words_of_the_program() {
    let mut engine = loaded(CHOR);
    engine.begin_undoable(UndoOp::FxFocusGroup);
    engine.fx_chains[SYNTH_A].mod2_type = Mod2Type::Phaser;
    engine.fx_chains[SYNTH_A].clear_mod2_firmware_words();
    assert_eq!(engine.fx_chains[SYNTH_A].mod2_type_raw, None);
    assert!(engine.undo().is_some());
    let fx = &engine.fx_chains[SYNTH_A];
    assert_eq!(fx.mod2_type, Mod2Type::Chorus);
    assert_eq!(fx.mod2_type_raw, Some(CHOR), "undo brings the stage back");
    assert!(fx.mod2_rate_raw.is_some());
}

#[test]
fn live_slot_recall_restores_the_words() {
    let mut engine = loaded(CHOR);
    let mut live = LiveSlots::default();
    engine.live_auto_store(&mut live, 1);
    engine.fx_chains[SYNTH_A].clear_mod2_firmware_words();
    assert!(engine.live_recall(&live, 1));
    assert_eq!(engine.fx_chains[SYNTH_A].mod2_type_raw, Some(CHOR));
    // a slot stored without words recalls without them
    engine.fx_chains[SYNTH_A].clear_mod2_firmware_words();
    engine.live_auto_store(&mut live, 2);
    engine.apply_program(&program(CHOR));
    assert!(engine.live_recall(&live, 2));
    assert_eq!(engine.fx_chains[SYNTH_A].mod2_type_raw, None);
}

#[test]
fn fx_copy_and_paste_carry_the_words() {
    let mut engine = loaded(CHOR);
    engine.copy_fx(LayerId::SynthA);
    engine.apply_program(&program(9)); // another program: SPIN, different stage
    assert_eq!(engine.fx_chains[SYNTH_A].mod2_type_raw, Some(9));
    engine.paste_fx(LayerId::SynthA).expect("paste");
    assert_eq!(engine.fx_chains[SYNTH_A].mod2_type_raw, Some(CHOR));
    assert_eq!(
        engine.fx_chains[SYNTH_A].mod2_rate_raw,
        Some((RATE * 127.0).round() as u8)
    );
}

#[test]
fn morph_of_the_rate_moves_the_firmware_rate_word() {
    let mut fx = port_chain(CHOR);
    fx.morph.mod2_rate = crate::morph::MorphParameter::new(0.2);
    fx.morph.mod2_rate.delta_wheel = 0.5;
    let ctrl = crate::morph::MorphControllers {
        mod_wheel: 1.0,
        ..Default::default()
    };
    fx.apply_morph(&ctrl);
    assert_eq!(fx.mod2_rate_raw, Some((0.7f32 * 127.0).round() as u8));
    let mut off = port_chain(CHOR);
    off.mod2_rate_raw = None; // engine path: the word stays absent
    off.morph = fx.morph;
    off.apply_morph(&ctrl);
    assert_eq!(off.mod2_rate_raw, None);
}
