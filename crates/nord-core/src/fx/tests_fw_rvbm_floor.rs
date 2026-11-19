//! The out-of-band floor of the firmware multi-tap reverb is a fixed fraction of the signal, not a
//! gain and not an absolute noise (`docs/research/reverb-stage-gain.md` s5): the +21 / +26 / +36 dB
//! "excess" of a firmware Cathedral pad render over its dry bands is the ratio of this floor to a
//! dry band that is nearly empty. MEASURED on the E40 port (equal to the oracle, `tests_fw_rvbm_long`).

use super::tests_fw_rvbm_tail::chain;
use super::*;

const INV_FS: f32 = 1.0 / 48_000.0;

/// Power above about 3 kHz (second difference) over total power of a 220 Hz tone of amplitude
/// `amp` through the firmware Cathedral at mix 1, left channel, after the first 0.34 s.
fn out_of_band_ratio(amp: f32) -> f64 {
    let mut fx = chain(true, ReverbModel::Cathedral, false);
    let y: Vec<f32> = (0..65_536)
        .map(|i| {
            let x = amp * (std::f32::consts::TAU * 220.0 * i as f32 / 48_000.0).sin();
            fx.process_reverb(x, x, INV_FS)[0]
        })
        .collect();
    let tail = &y[16_384..];
    let hp: f64 = tail
        .windows(3)
        .map(|w| f64::from(w[0] - 2.0 * w[1] + w[2]).powi(2))
        .sum();
    let tot: f64 = tail.iter().map(|v| f64::from(*v).powi(2)).sum();
    hp / tot
}

/// The floor scales with the signal: the out-of-band to total power ratio is the same within
/// 0.1 dB between amplitude 0.3 and 0.03 and within 2 dB at 0.003 (measured -55.75, -55.75,
/// -54.19 dB: a small absolute part appears at the lowest level). An absolute noise floor would
/// rise by 20 dB per decade of attenuation.
#[test]
fn the_out_of_band_floor_is_a_fixed_fraction_of_the_signal() {
    let r: Vec<f64> = [0.3f32, 0.03, 0.003]
        .iter()
        .map(|&a| 10.0 * out_of_band_ratio(a).log10())
        .collect();
    assert!(
        (r[0] - r[1]).abs() < 0.1 && (r[0] - r[2]).abs() < 2.0,
        "{r:?} dB"
    );
    assert!(r[0] < -20.0, "{r:?} dB");
}

/// A fixed fraction of the signal is invisible against a dry band that carries comparable energy
/// and large against an empty one: for a dry band `d` dB below the tone, the wet band reads
/// `floor - d` dB over the dry, which is how a pad with an empty top octave shows +20 to +36 dB.
#[test]
fn an_empty_dry_band_turns_a_low_floor_into_a_large_ratio() {
    let floor = 10.0 * out_of_band_ratio(0.3).log10();
    let excess = |dry_below_tone: f64| floor + dry_below_tone;
    assert!(
        excess(40.0) < 0.0,
        "a dry band 40 dB below the tone hides the floor"
    );
    assert!(
        excess(100.0) > 20.0,
        "a dry band 100 dB below the tone shows +{} dB",
        excess(100.0)
    );
}
