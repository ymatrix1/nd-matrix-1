//! Level, linearity and tail facts of the firmware multi-tap reverb in the layer chain
//! (`docs/research/reverb-fw-vs-hw.md`): why the firmware path sounds quieter, shorter and darker
//! than the engine FDN on the same type and knob, and that the routine adds no gain of its own.
//! MEASURED on the E40 port (equal to the oracle over 30,000 calls, `tests_fw_rvbm_long.rs`).

use super::tests_fw_rvbm_tail::{chain, chain_t60};
use super::*;

const INV_FS: f32 = 1.0 / 48_000.0;

/// Energy of the left output of an impulse after the first `skip` frames (the dry impulse).
fn wet_energy(fx: &mut LayerFxChain, frames: usize, skip: usize) -> f64 {
    (0..frames)
        .map(|k| {
            let x = f32::from(k == 0);
            let y = fx.process_reverb(x, x, INV_FS)[0];
            if k < skip {
                0.0
            } else {
                f64::from(y).powi(2)
            }
        })
        .sum()
}

/// Output power of `fx` for deterministic noise of amplitude `amp`.
fn noise_out_power(fx: &mut LayerFxChain, amp: f32) -> f64 {
    let mut s = 0x2545_F491u32;
    let mut acc = 0.0f64;
    for k in 0..40_000 {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let x = amp * (((s >> 8) as f32 - 8_388_608.0) / 8_388_608.0);
        let y = fx.process_reverb(x, x, INV_FS)[0];
        if k >= 4_000 {
            acc += f64::from(y).powi(2);
        }
    }
    // uniform noise of amplitude `amp` has power `amp^2 / 3`; 36,000 frames are summed
    acc / 36_000.0 / (f64::from(amp).powi(2) / 3.0)
}

/// The routine is linear in level (no gain, no compression): output power over input power
/// differs by under 0.1 dB between amplitude 0.01 and 1.0.
#[test]
fn the_firmware_reverb_gain_does_not_depend_on_the_input_level() {
    let (mut a, mut b) = (
        chain(true, ReverbModel::Cathedral, false),
        chain(true, ReverbModel::Cathedral, false),
    );
    let (ga, gb) = (noise_out_power(&mut a, 0.01), noise_out_power(&mut b, 1.0));
    assert!((10.0 * (ga / gb).log10()).abs() < 0.1, "{ga} {gb}");
    assert!(ga < 0.35, "wet-only power gain stays below unity: {ga}");
}

/// At the same knob the firmware wet energy is far below the engine FDN wet energy (Hall,
/// mix 0.3): the firmware wet send is `x^2 (2 - x)` of the unit mix, the FDN's is not.
#[test]
fn at_a_low_knob_the_firmware_wet_is_over_ten_db_below_the_engine_wet() {
    let wet = |fw: bool| {
        let mut fx = chain(fw, ReverbModel::Hall, false);
        fx.reverb_mix = 0.3;
        wet_energy(&mut fx, 96_000, 64)
    };
    let (e_fw, e_fdn) = (wet(true), wet(false));
    let d = 10.0 * (e_fdn / e_fw).log10();
    assert!(d > 12.0 && d < 18.0, "engine over firmware: {d} dB");
}

/// Hall and Cathedral have the same firmware tail (1.2 s), unlike the engine FDN (3.4 s and
/// 7.9 s): the type words differ in length and voicing words, not in the decay.
#[test]
fn firmware_hall_and_cathedral_tails_are_equal() {
    let hall = chain_t60(&mut chain(true, ReverbModel::Hall, false), 96_000, 64);
    let cath = chain_t60(&mut chain(true, ReverbModel::Cathedral, false), 96_000, 64);
    assert!((hall - cath).abs() < 0.05, "{hall} {cath}");
    assert!((cath - 1.19).abs() < 0.05, "{cath}");
}

/// A 220 Hz tone through the firmware Cathedral (mix 1) leaves a floor of the packed delay words:
/// loose bounds on the measured -51 / -61 / -70 dB (sub, brilliance, air) re the tone.
#[test]
fn a_pure_tone_leaves_a_low_floor_outside_its_band() {
    let mut fx = chain(true, ReverbModel::Cathedral, false);
    let mut y = Vec::new();
    for i in 0..65_536 {
        let x = 0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / 48_000.0).sin();
        y.push(fx.process_reverb(x, x, INV_FS)[0]);
    }
    // power above 3 kHz via a first difference twice (crude high-pass) against total power
    let tail = &y[16_384..];
    let hp: f64 = tail
        .windows(3)
        .map(|w| f64::from(w[0] - 2.0 * w[1] + w[2]).powi(2))
        .sum();
    let tot: f64 = tail.iter().map(|v| f64::from(*v).powi(2)).sum();
    // the second difference gains up to +12 dB at 3 kHz and +24 dB at Nyquist; 220 Hz is -37 dB
    assert!(10.0 * (hp / tot).log10() < -20.0, "{}", hp / tot);
}
