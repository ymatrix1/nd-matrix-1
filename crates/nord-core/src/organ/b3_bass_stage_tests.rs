//! Tests of the firmware Bass stage (`b3_bass_stage.rs`).

use super::b3_bass_fw::{real_image, synthetic_image, BassImage};
use super::b3_bass_stage::B3BassStage;
use super::*;

const FS: f32 = 48_000.0;
const GOLDENS: &str = include_str!("../../../../docs/research/data/bass-chord-goldens.csv");

fn layer(image: &BassImage) -> OrganLayer {
    let mut o = OrganLayer::new(FS);
    o.model = OrganModel::B3;
    o.b3_bass_mode = true;
    o.tonewheel_source = ToneWheelSource::B3Wheels;
    o.set_b3_bass_processor_with(true, Some(image));
    o.rotary_enabled = false;
    o.key_click_enabled = false;
    o.percussion_enabled = false;
    o
}

fn render(o: &mut OrganLayer, n: usize) -> Vec<f32> {
    (0..n).map(|_| o.render_frame()[0]).collect()
}

/// Peak bin of an `x.len()` point Hann DFT near `hz` (the oracle's line amplitude method,
/// `bass-chord-goldens.csv`: 8192 points at the DSP rate).
fn hann_line(x: &[f32], hz: f64, fs: f64) -> f32 {
    let n = x.len();
    let centre = (hz * n as f64 / fs).round() as i64;
    let win: f64 = (0..n)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos())
        .sum();
    let bin = |k: i64| {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let h = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            let a = std::f64::consts::TAU * k as f64 * i as f64 / n as f64;
            re += f64::from(*v) * h * a.cos();
            im -= f64::from(*v) * h * a.sin();
        }
        2.0 * re.hypot(im) / win
    };
    (centre - 1..=centre + 1).map(bin).fold(0.0f64, f64::max) as f32
}

/// Worst `20 log10(stage / oracle)` over the lines of golden chord `name`.
fn chord_error_db(image: &BassImage, name: &str) -> f32 {
    let rows: Vec<Vec<&str>> = GOLDENS
        .lines()
        .skip(1)
        .map(|l| l.split(',').collect())
        .filter(|f: &Vec<&str>| f[0] == name)
        .collect();
    let mut stage = B3BassStage::new(image);
    for k in rows[0][1].split(' ').map(|k| k.parse::<usize>().unwrap()) {
        stage.note_on(k, k as i32);
    }
    let x = stage.dsp_block(1_200 + 8_192);
    let seg = &x[1_200..];
    rows.iter()
        .map(|f| {
            let hz: f64 = f[3].parse().unwrap();
            let want: f32 = f[5].parse().unwrap();
            (20.0 * (hann_line(seg, hz, 43_402.6) / want).log10()).abs()
        })
        .fold(0.0, f32::max)
}

#[test]
fn stage_is_off_by_default_and_needs_bass_mode_and_b3_wheels() {
    let o = OrganLayer::new(FS);
    assert!(!o.b3_bass_active());
    let mut o = layer(&synthetic_image());
    assert!(o.b3_bass_active());
    o.b3_bass_mode = false;
    assert!(!o.b3_bass_active());
    o.b3_bass_mode = true;
    o.set_b3_tail(true);
    assert!(!o.b3_bass_active());
}

#[test]
fn without_an_image_the_stage_is_not_built_and_the_layer_keeps_its_tail() {
    // D128: no OS update file, no firmware stage; the layer plays the stage-off model (D22)
    let mut o = OrganLayer::new(FS);
    o.model = OrganModel::B3;
    o.b3_bass_mode = true;
    o.tonewheel_source = ToneWheelSource::B3Wheels;
    o.set_b3_tail(true);
    let before = (0..200).map(|_| o.render_frame()[0]).collect::<Vec<f32>>();
    o.set_b3_bass_processor_with(true, None);
    assert!(o.b3_bass_processor_on(), "the request is kept");
    assert!(!o.b3_bass_firmware_loaded() && !o.b3_bass_active() && !o.b3_bass_sounding());
    assert!(o.b3_tail, "the tail stays: no stage replaces it");
    let mut off = OrganLayer::new(FS);
    off.model = OrganModel::B3;
    off.b3_bass_mode = true;
    off.tonewheel_source = ToneWheelSource::B3Wheels;
    off.set_b3_tail(true);
    let reference = (0..200)
        .map(|_| off.render_frame()[0])
        .collect::<Vec<f32>>();
    assert_eq!(before, reference, "idle renders are identical");
    o.set_b3_bass_processor_with(true, Some(&synthetic_image()));
    assert!(o.b3_bass_firmware_loaded() && !o.b3_tail);
}

#[test]
fn single_key_and_chord_lines_follow_the_oracle_goldens() {
    // needs the user's OS update file (`NS4_WAVE_TABLES`); skipped with a note without it
    let Some(image) = real_image() else { return };
    // `bass-chord-goldens.csv`: the oracle (`sharc-chain`, boot + model 5 install + the ARM
    // key-down, 8192 point Hann lines of the output stream). The FIR stage read +1.1 .. +2.5 dB on
    // the chord 60 64 67 (research s12); the firmware recursion reads within the bound below.
    // Measured worst line error per chord is in research s15.
    for (name, bound) in [
        ("g1", 0.05),
        ("g3", 0.05),
        ("g3r", 0.05),
        ("g10", 0.05),
        ("g10r", 0.05),
        ("g10s", 0.05),
    ] {
        let worst = chord_error_db(&image, name);
        assert!(worst < bound, "{name}: worst line error {worst} dB");
    }
}

#[test]
fn resampled_output_is_finite_audible_and_keeps_the_dsp_amplitude() {
    // the host rate stream (cubic resampler, 43,402.6 -> 48,000 Hz) of key 60 has the amplitude of
    // the DSP rate stream (peak of both within 5 %); the synthetic routine is a constant output
    for image in [Some(synthetic_image().into()), real_image()]
        .into_iter()
        .flatten()
        .collect::<Vec<std::sync::Arc<BassImage>>>()
    {
        let mut raw = B3BassStage::new(&image);
        raw.note_on(60, 60);
        let dsp = raw.dsp_block(8_000);
        let dsp_peak = dsp[2_000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let mut stage = B3BassStage::new(&image);
        stage.note_on(60, 60);
        let x = (0..9_600).map(|_| stage.process(FS)).collect::<Vec<_>>();
        assert!(x.iter().all(|v| v.is_finite()));
        assert!(x.iter().any(|v| v.abs() > 1e-4));
        let peak = x[2_400..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak / dsp_peak - 1.0).abs() < 0.05, "{peak} vs {dsp_peak}");
    }
}

#[test]
fn release_rings_down_to_silence_and_stays_finite() {
    for image in [Some(synthetic_image().into()), real_image()]
        .into_iter()
        .flatten()
        .collect::<Vec<std::sync::Arc<BassImage>>>()
    {
        let mut o = layer(&image);
        o.note_on(48);
        o.note_on(55);
        let held = render(&mut o, 4_000);
        assert!(held.iter().all(|v| v.is_finite()));
        assert!(held.iter().any(|v| v.abs() > 1e-4));
        o.note_off(48);
        o.note_off(55);
        let tail = render(&mut o, 30_000);
        assert!(tail.iter().all(|v| v.is_finite()));
        assert!(tail[29_000..].iter().all(|v| v.abs() < 1e-7));
        assert!(!o.b3_bass_sounding());
    }
}

#[test]
fn octave_shift_registers_the_transposed_key_and_releases_it() {
    let mut o = layer(&synthetic_image());
    o.octave_shift = 1;
    o.note_on(60);
    o.octave_shift = 0;
    o.note_off(60);
    let x = render(&mut o, 30_000);
    assert!(x[29_000..].iter().all(|v| v.abs() < 1e-7));
    // a key outside the decoded range 24..=108 does nothing (the stage stays idle)
    o.note_on(10);
    render(&mut o, 2_000);
    assert!(!o.b3_bass_sounding());
}
