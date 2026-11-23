//! Tests of the firmware Farfisa stage (`farfisa_keys.rs`).

use super::farfisa_fw::{real_image, FarfisaImage};
use super::farfisa_keys::FarfisaKeys;
use super::*;

const FS: f32 = 48_000.0;
const GOLDENS: &str = include_str!("../../../../docs/research/data/farfisa-chord-goldens.csv");

/// A layer with the stage switched `processor` on `image` (`None`: no OS update file).
fn layer(processor: bool, image: Option<&FarfisaImage>, model: OrganModel) -> OrganLayer {
    let mut o = OrganLayer::new(FS);
    o.model = model;
    o.tonewheel_source = ToneWheelSource::FarfisaWheels;
    o.set_farfisa_processor_with(processor, image);
    o.rotary_enabled = false;
    o.key_click_enabled = false;
    o.percussion_enabled = false;
    o
}

/// Peak bin of an `x.len()` point Hann DFT near `hz` (the oracle's line method, as the Bass test).
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

/// `(worst, mean)` of `|20 log10(stage / oracle)|` over the lines of golden chord `name`.
fn chord_error_db(image: &FarfisaImage, name: &str) -> (f32, f32) {
    let rows: Vec<Vec<&str>> = GOLDENS
        .lines()
        .skip(1)
        .map(|l| l.split(',').collect())
        .filter(|f: &Vec<&str>| f[0] == name)
        .collect();
    let mut stage = FarfisaKeys::new(image);
    for k in rows[0][1].split(' ').map(|k| k.parse::<usize>().unwrap()) {
        stage.note_on(k, k as i32, 1.0);
    }
    let x = stage.dsp_block(1_200 + 8_192);
    let errs: Vec<f32> = rows
        .iter()
        .map(|f| {
            let hz: f64 = f[2].parse().unwrap();
            let want: f32 = f[3].parse().unwrap();
            (20.0 * (hann_line(&x[1_200..], hz, 43_402.6) / want).log10()).abs()
        })
        .collect();
    (
        errs.iter().cloned().fold(0.0, f32::max),
        errs.iter().sum::<f32>() / errs.len() as f32,
    )
}

#[test]
fn chord_lines_follow_the_oracle_goldens() {
    // needs the user's OS update file (`NS4_WAVE_TABLES`); skipped with a note without it
    let Some(image) = real_image() else { return };
    // `farfisa-chord-goldens.csv`: the 12 strongest 8192 point Hann lines of the oracle output
    // (`sharc-chain`, boot + model 2 install + key-down; generator + processor) of four chords.
    // The gain-table FIR stage was within 2.9 dB (research s9); the recursion on the divider of
    // `FarfisaWheels` (ring within 2e-4 of the generator) reads within the bound below.
    // The measured worst error per chord is in research s16.
    for name in ["k1", "k3", "k48", "k72"] {
        let (worst, mean) = chord_error_db(&image, name);
        println!("{name}: worst {worst:.4} dB, mean {mean:.4} dB");
        assert!(worst < 0.02, "{name}: worst line error {worst} dB");
    }
}

#[test]
fn stage_is_off_by_default_and_needs_the_farfisa_source() {
    let o = OrganLayer::new(FS);
    assert!(!o.farfisa_processor && !o.farfisa_keys_active());
    let image = synthetic_image();
    let mut o = layer(true, Some(&image), OrganModel::Farfisa);
    assert!(o.farfisa_keys_active() && o.farfisa_firmware_loaded());
    o.tonewheel_source = ToneWheelSource::B3Wheels;
    assert!(!o.farfisa_keys_active(), "needs the FarfisaWheels source");
    let mut o = layer(true, Some(&image), OrganModel::B3);
    o.tonewheel_source = ToneWheelSource::FarfisaWheels;
    assert!(!o.farfisa_keys_active(), "Farfisa model only");
}

fn synthetic_image() -> FarfisaImage {
    super::farfisa_fw::synthetic_image()
}

fn render_peak(processor: bool, image: Option<&FarfisaImage>, model: OrganModel) -> f32 {
    let mut o = layer(processor, image, model);
    o.note_on(60);
    let mut peak = 0.0f32;
    for i in 0..24_000 {
        let [l, _] = o.render_frame();
        if i > 12_000 {
            peak = peak.max(l.abs());
        }
    }
    peak
}

#[test]
fn wiring_is_opt_in_and_only_for_the_farfisa_model() {
    let Some(image) = real_image() else { return };
    let off = render_peak(false, None, OrganModel::Farfisa);
    let on = render_peak(true, Some(&image), OrganModel::Farfisa);
    assert!(on > 1e-3 && off > 1e-3, "{on} {off}");
    assert!(
        (on - off).abs() > 1e-4,
        "the stage changes the Farfisa render"
    );
    assert_eq!(
        render_peak(true, Some(&image), OrganModel::B3),
        render_peak(false, None, OrganModel::B3)
    );
}

#[test]
fn without_an_os_update_file_the_stage_falls_back_to_the_non_firmware_model() {
    // D22 / D128: switched on without an image the firmware stage does not exist and the layer
    // renders exactly what it renders with the stage off (the Legacy Farfisa model).
    let mut on = layer(true, None, OrganModel::Farfisa);
    let mut off = layer(false, None, OrganModel::Farfisa);
    assert!(on.farfisa_processor, "the request is kept");
    assert!(!on.farfisa_firmware_loaded() && !on.farfisa_keys_active());
    on.note_on(60);
    off.note_on(60);
    for i in 0..6_000 {
        let (a, b) = (on.render_frame(), off.render_frame());
        assert_eq!(a, b, "frame {i}");
    }
    assert!(!on.farfisa_keys_sounding());
}

#[test]
fn release_rings_down_to_silence_and_stays_finite() {
    let Some(image) = real_image() else { return };
    let mut o = layer(true, Some(&image), OrganModel::Farfisa);
    o.note_on(60);
    for _ in 0..4_000 {
        o.render_frame();
    }
    o.note_off(60);
    let mut last = 1.0f32;
    for _ in 0..80_000 {
        let [l, r] = o.render_frame();
        assert!(l.is_finite() && r.is_finite());
        last = l;
    }
    assert_eq!(last, 0.0, "the stage idles after the ring-down");
    assert!(!o.farfisa_keys_sounding());
}

#[test]
fn octave_shift_registers_the_transposed_key_and_out_of_range_keys_are_ignored() {
    let image = synthetic_image();
    let mut o = layer(true, Some(&image), OrganModel::Farfisa);
    o.octave_shift = 2;
    o.note_on(60); // key 84
    o.note_off(60);
    let mut s = FarfisaKeys::new(&image);
    s.note_on(10, 10, 1.0); // below the key table
    assert!(!s.active());
    s.note_on(60, 60, 1.0);
    assert!(s.active());
    s.note_off(60);
    assert!(s.active(), "rings down before idling");
}

#[test]
fn velocity_gain_scales_the_output_linearly() {
    let Some(image) = real_image() else { return };
    let run = |g: f32| {
        let mut s = FarfisaKeys::new(&image);
        s.note_on(60, 60, g);
        let x = s.dsp_block(6_000);
        x[3_000..].iter().fold(0.0f32, |m, v| m.max(v.abs()))
    };
    let (a, b) = (run(1.0), run(0.5));
    assert!(a > 1e-3 && ((b / a) - 0.5).abs() < 1e-3, "{a} {b}");
}
