//! Osc Env Amount under Env To Pitch (OQ44, D214): the firmware pitch law against the ORG core 2
//! oracle rows, and the opt-in engine path.

use crate::engine::Stage4Engine;
use crate::synth::SynthLayer;
use crate::synth_osc_env_word::{osc_env_dsp_word, osc_env_pitch_semitones};

/// Oracle rows of `tools/synth_osc_env_pitch.py law`: DSP word 12, envelope level, measured and
/// modelled semitones.
const ROWS: &str = include_str!("../../../docs/research/data/synth-osc-env-pitch-oracle.csv");

/// Amount steps whose Env To Pitch DSP word is `word` (the law is odd and monotone: bisection).
fn steps_of_word(word: f32) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 60.0f32);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if osc_env_dsp_word(mid, true) < word.abs() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (0.5 * (lo + hi)).copysign(word)
}

#[test]
fn pitch_law_matches_every_oracle_row() {
    let mut n = 0;
    let mut worst = 0.0f32;
    for line in ROWS.lines().skip(1) {
        let c: Vec<f32> = line.split(',').map(|v| v.parse().unwrap()).collect();
        let (word, level, oracle) = (c[0], c[1], c[2]);
        let st = osc_env_pitch_semitones(steps_of_word(word), level);
        worst = worst.max((st - oracle).abs());
        n += 1;
    }
    assert_eq!(
        n, 46,
        "12 words x 4 levels, minus 2 rows below the 40 Hz estimator floor"
    );
    // The FFT peak estimator limits the oracle rows to about 0.2 semitones.
    assert!(worst < 0.25, "worst {worst} st");
}

/// Rising zero crossings of the layer output after the attack.
fn crossings(amount: f32, to_pitch: bool, fw: bool, note: u8) -> usize {
    let mut s = SynthLayer::legacy(48_000.0);
    s.filter_on = false;
    s.osc_env_to_pitch = to_pitch;
    s.osc_env_amount = amount;
    s.osc_env_fw_pitch = fw;
    s.osc_env_attack_sec = 0.001;
    s.osc_env_decay_sec = 60.0; // no decay: the envelope stays at 1
    s.note_on(note, 100);
    for _ in 0..4_800 {
        s.render_frame();
    }
    let mut prev = 0.0f32;
    let mut n = 0;
    for _ in 0..48_000 {
        let x = s.render_frame()[0];
        if prev <= 0.0 && x > 0.0 {
            n += 1;
        }
        prev = x;
    }
    n
}

#[test]
fn fw_pitch_path_plays_48_semitones_at_full_amount_and_follows_the_sign() {
    let base = crossings(0.0, true, true, 48) as f32;
    let up = crossings(1.0, true, true, 48) as f32;
    let down = crossings(-1.0, true, true, 48) as f32;
    assert!((up / base - 16.0).abs() < 1.6, "up {up} base {base}");
    assert!(down < base * 0.25, "down {down} base {base}");
}

#[test]
fn fw_pitch_path_is_off_by_default_and_leaves_osc_ctrl_alone() {
    let lin = crossings(0.25, true, false, 48) as f32;
    let fw = crossings(0.25, true, true, 48) as f32;
    // Linear law: 0.25 oct (3 st); firmware taper at 15 of 60 steps: 3.4 st. Both sound close.
    assert!((lin - fw).abs() / lin < 0.2);
    assert_eq!(
        crossings(0.7, false, false, 48),
        crossings(0.7, false, true, 48),
        "Osc Ctrl destination ignores the flag"
    );
}

#[test]
fn engine_sets_the_layer_flag_from_the_morph_option() {
    let mut e = Stage4Engine::new(48_000.0);
    e.render_frame();
    assert!(!e.synth_a.osc_env_fw_pitch, "default off");
    e.synth_osc_env_morph = true;
    e.render_frame();
    assert!(e.synth_a.osc_env_fw_pitch && e.synth_b.osc_env_fw_pitch && e.synth_c.osc_env_fw_pitch);
    e.firmware_morph = false;
    e.render_frame();
    assert!(
        !e.synth_a.osc_env_fw_pitch,
        "--legacy-morph keeps the old engine"
    );
}
