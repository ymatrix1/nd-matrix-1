//! Tail time (T60) of the reverb types in the layer chain: the firmware multi-tap routine against
//! the engine FDN, at the engine rate of 48 kHz.

use super::*;

const FS: f64 = 48_000.0;

/// T60 (s) of the wet impulse response of the chain, from the Schroeder backward integral fitted
/// between -5 and -25 dB (T20 extrapolated). The first `skip` frames (the dry impulse) are left out.
pub(super) fn chain_t60(fx: &mut LayerFxChain, frames: usize, skip: usize) -> f64 {
    let mut e = Vec::with_capacity(frames);
    for k in 0..frames {
        let x = f32::from(k == 0);
        let y = fx.process_reverb(x, x, 1.0 / 48_000.0);
        e.push(if k < skip {
            0.0
        } else {
            f64::from(y[0]).powi(2)
        });
    }
    let mut acc = 0.0;
    let mut edc = vec![0.0f64; frames];
    for (i, v) in e.iter().enumerate().rev() {
        acc += v;
        edc[i] = acc;
    }
    let total = edc[skip];
    let at = |level: f64| {
        edc.iter()
            .enumerate()
            .skip(skip)
            .find(|(_, &v)| 10.0 * (v / total).log10() <= level)
            .map(|(i, _)| i)
    };
    match (at(-5.0), at(-25.0)) {
        (Some(a), Some(b)) => 3.0 * (b - a) as f64 / FS,
        _ => f64::NAN,
    }
}

pub(super) fn chain(fw: bool, model: ReverbModel, variation: bool) -> LayerFxChain {
    let mut fx = LayerFxChain::new(48_000.0);
    fx.reverb_model = model;
    fx.reverb_variation = variation;
    fx.reverb_mix = 1.0;
    fx.firmware_fx = FirmwareFx {
        reverb: fw,
        ..FirmwareFx::OFF
    };
    fx
}

/// MEASURED (this test): the firmware tails of Room and Hall against the engine FDN. The FDN
/// Hall is nearly three times the firmware routine and the FDN Cathedral (7.9 s, not run here for
/// speed) is over six times; neither is verified against the hardware, so the FDN stays the
/// default (D22, D91).
#[test]
fn the_engine_fdn_hall_is_much_longer_than_the_firmware_hall_and_room_is_close() {
    let fw_hall = chain_t60(&mut chain(true, ReverbModel::Hall, false), 96_000, 64);
    let fdn_hall = chain_t60(&mut chain(false, ReverbModel::Hall, false), 288_000, 64);
    assert!((fw_hall - 1.202).abs() < 0.04, "{fw_hall}");
    assert!((fdn_hall - 3.437).abs() < 0.10, "{fdn_hall}");
    let fw_room = chain_t60(&mut chain(true, ReverbModel::Room, false), 96_000, 64);
    let fdn_room = chain_t60(&mut chain(false, ReverbModel::Room, false), 96_000, 64);
    assert!((fw_room - 0.748).abs() < 0.03, "{fw_room}");
    assert!((fdn_room - 0.598).abs() < 0.03, "{fdn_room}");
}

#[test]
fn a_dark_colour_changes_the_chain_output_of_the_firmware_reverb() {
    let run = |color| {
        let mut fx = chain(true, ReverbModel::Hall, false);
        fx.reverb_color = color;
        (0..20_000)
            .map(|k| fx.process_reverb(f32::from(k == 0), f32::from(k == 0), 1.0 / 48_000.0)[0])
            .collect::<Vec<_>>()
    };
    let (n, d, b, c) = (
        run(ReverbColor::Normal),
        run(ReverbColor::Dark),
        run(ReverbColor::Bright),
        run(ReverbColor::Chorale),
    );
    assert!(
        n.iter().any(|v| v.abs() > 1e-6),
        "a tail inside 20,000 frames"
    );
    assert!(n != d, "Dark differs from Normal");
    assert!(d != b, "Bright differs from Dark");
    assert!(n == c, "Chorale keeps colour 0");
}
