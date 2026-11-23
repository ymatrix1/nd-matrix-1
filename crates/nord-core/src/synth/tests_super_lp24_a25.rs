//! A:25 Sidechain Whl II synth layer (SUPER wave 0, LP24 1600 Hz): the layer-level effect of the
//! `SUPER_LP24_TYPE_SCALE` default against `--synth-super-lp24 2.0` (D221,
//! `docs/research/a25-brilliance-bisect.md`).

use super::p_table::PWave;
use super::*;

/// The A:25 synth layer A on the strip chain with the table row, LP24 at 1600 Hz, resonance 0.1.
fn a25_layer(super_lp24: Option<f32>) -> SynthLayer {
    let mut s = SynthLayer::new(48_000.0);
    s.chain = SynthChain::FirmwareStrip;
    s.waveform = VaWaveform::SuperSaw;
    s.p_table = Some(PTableSpec::BUILTIN.table());
    s.p_wave = PWave::Super;
    s.filter_model = FilterModel::Lp24;
    s.cutoff_hz = 1_600.0;
    s.resonance = 0.1;
    s.attack_sec = 0.002;
    s.decay_sec = 45.0;
    s.sustain_level = 1.0;
    s.release_sec = 0.05;
    s.super_lp24_scale = super_lp24;
    s
}

/// Power share (dB re total) above `split_hz` of a steady note, naive DFT of the left channel.
fn high_share_db(s: &mut SynthLayer, note: u8, split_hz: f32) -> f32 {
    s.note_on(note, 100);
    for _ in 0..4_000 {
        let _ = s.render_frame();
    }
    let n = 2_048usize;
    let frames: Vec<f32> = (0..n).map(|_| s.render_frame()[0]).collect();
    let (mut hi, mut all) = (0.0f64, 0.0f64);
    for k in 1..n / 2 {
        let w = std::f64::consts::TAU * k as f64 / n as f64;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, x) in frames.iter().enumerate() {
            re += f64::from(*x) * (w * i as f64).cos();
            im -= f64::from(*x) * (w * i as f64).sin();
        }
        let p = re * re + im * im;
        all += p;
        if k as f32 * 48_000.0 / n as f32 > split_hz {
            hi += p;
        }
    }
    (10.0 * (hi.max(1e-30) / all.max(1e-30)).log10()) as f32
}

#[test]
fn default_type_scale_is_the_a25_brightness_source_and_2_0_removes_it() {
    // EMPIRICAL (A:25 clip, `emu_vs_hw_a25_super_results.json`): the layer at 4.4 carries 8 dB
    // more 3..8 kHz power than at 2.0. The unit pins the direction and a floor, not the 8 dB.
    let note = 60;
    let dflt = high_share_db(&mut a25_layer(None), note, 3_000.0);
    let two = high_share_db(&mut a25_layer(Some(2.0)), note, 3_000.0);
    assert!(dflt.is_finite() && two.is_finite());
    assert!(
        dflt > two + 3.0,
        "4.4 default {dflt:.1} dB against 2.0 {two:.1} dB above 3 kHz"
    );
}

#[test]
fn explicit_4_4_equals_the_default() {
    let a = high_share_db(&mut a25_layer(None), 60, 3_000.0);
    let b = high_share_db(
        &mut a25_layer(Some(super::p_type::SUPER_LP24_TYPE_SCALE)),
        60,
        3_000.0,
    );
    assert_eq!(a, b);
}
