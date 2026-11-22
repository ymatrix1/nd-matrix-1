//! `FirmwareFx::compressor_first` (D214): the firmware compressor runs ahead of the amp / LP24
//! stage. Opt-in, so the default order and render stay as they were.

use super::*;

fn chain(first: bool) -> LayerFxChain {
    let mut fx = LayerFxChain::new(48_000.0);
    fx.reverb_model = ReverbModel::Off;
    fx.amp_cabinet = AmpCabinet::LpFilter;
    fx.eq_mid_freq_hz = 2000.0;
    fx.eq_filter_res = 3.0;
    fx.comp_enabled = true;
    fx.comp_amount = 0.5;
    fx.firmware_fx = FirmwareFx {
        compressor: true,
        compressor_row: true,
        compressor_first: first,
        ..FirmwareFx::OFF
    };
    fx
}

/// A 220 Hz tone whose level steps between 0.5 and 0.03 every 1,500 frames (a note onset and a
/// decay: the gain computer has to move).
fn run(fx: &mut LayerFxChain, n: usize) -> Vec<f32> {
    (0..n)
        .map(|k| {
            let amp = if (k / 1_500) % 2 == 0 { 0.5 } else { 0.03 };
            let x = amp * (std::f32::consts::TAU * 220.0 * k as f32 / 48_000.0).sin();
            fx.process_frame([x, x])[0]
        })
        .collect()
}

/// Energy of the second difference (a high-pass, about +12 dB per octave) of the second half.
fn hf_energy(y: &[f32]) -> f64 {
    y[y.len() / 2..]
        .windows(3)
        .map(|w| f64::from(w[2] - 2.0 * w[1] + w[0]).powi(2))
        .sum()
}

#[test]
fn compressor_first_is_off_by_default_and_not_in_all_or_the_port_list() {
    let (d, all) = (
        std::hint::black_box(FirmwareFx::DEFAULT),
        std::hint::black_box(FirmwareFx::ALL),
    );
    assert!(!d.compressor_first && !all.compressor_first);
    let mut g = FirmwareFx::OFF;
    g.merge_on(&FirmwareFx {
        compressor_first: true,
        ..FirmwareFx::OFF
    });
    assert!(g.compressor_first, "merge_on carries it");
}

#[test]
fn the_default_order_is_unchanged_and_the_flag_moves_the_gain_steps_before_the_low_pass() {
    let n = 24_000;
    let (after, first) = (run(&mut chain(false), n), run(&mut chain(true), n));
    assert!(after.iter().all(|v| v.is_finite()) && first.iter().all(|v| v.is_finite()));
    let (ea, ef) = (hf_energy(&after), hf_energy(&first));
    assert!(
        ef < 0.5 * ea,
        "compressor before the 2 kHz LP24 leaves less high-frequency energy: first {ef:.3e}, after {ea:.3e}"
    );
    let diff: f32 = after.iter().zip(&first).map(|(a, b)| (a - b).abs()).sum();
    assert!(diff > 1.0, "the order changes the output: {diff}");
}

#[test]
fn with_the_firmware_compressor_off_the_flag_changes_nothing() {
    let mut a = chain(false);
    let mut b = chain(true);
    a.firmware_fx.compressor = false;
    b.firmware_fx.compressor = false;
    assert_eq!(run(&mut a, 6_000), run(&mut b, 6_000));
}

#[test]
fn the_cli_flag_sets_it_and_the_summary_names_it() {
    use crate::render_opts::FirmwareOptions;
    let d = FirmwareOptions::default();
    assert!(!d.fx.compressor_first && !d.summary().contains("fx-compressor-first"));
    let o = FirmwareOptions::from_args(&["--fx-compressor-first".into()]).unwrap();
    assert!(o.fx.compressor_first && o.summary().contains("fx-compressor-first"));
    assert!(
        !FirmwareOptions::firmware_all().fx.compressor_first,
        "--firmware-all does not include it"
    );
}
