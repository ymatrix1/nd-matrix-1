//! FX delay-rate morph (`FxMorphSet::delay_rate`, D204): the morphed 14 bit value reaches the
//! firmware delay time law, which the opt-in `FirmwareFx::delay_time` reads.
//!
//! Fixture: `tools/arm_program_morph.py --dsp-all` on a factory program with delay-rate depths
//! `+12 / +24 / -36` steps of 127 (`tools/morph_test_program.py`), base 11008:
//! `docs/research/data/morph-fw-delay.csv` (dispatched edit value, DSP word `c[0x15]` at DM
//! `0xB28AF`).

use super::*;
use crate::morph::MorphControllers;

const BASE: u16 = 11_008;
/// Depth steps of the three sources on the 16383 scale: `12/127`, `24/127`, `-36/127` of the range.
const DEPTH: [f32; 3] = [1548.0, 3096.0, -4644.0];

/// `(source, MIDI value, dispatched value, DSP word)` rows of `morph-fw-delay.csv`.
const ARM: [(usize, u8, f64, f32); 10] = [
    (0, 1, 11020.0938, 10803.0),
    (0, 5, 11068.4688, 10609.0),
    (0, 33, 11407.0938, 9331.0),
    (0, 64, 11782.0, 7734.0),
    (0, 100, 12217.375, 6661.0),
    (0, 127, 12556.0, 6007.0),
    (1, 1, 11032.1875, 10755.0),
    (1, 64, 12556.0, 6007.0),
    (1, 127, 14104.0, 2913.0),
    (2, 127, 6364.0, 20231.0),
];

fn chain() -> LayerFxChain {
    let mut fx = LayerFxChain::new(48000.0);
    fx.delay_rate_raw = Some(BASE);
    let m = &mut fx.morph.delay_rate;
    *m = MorphParameter::new(f32::from(BASE) / 16383.0);
    m.delta_wheel = DEPTH[0] / 16383.0;
    m.delta_aftertouch = DEPTH[1] / 16383.0;
    m.delta_pedal = DEPTH[2] / 16383.0;
    fx
}

fn controllers(source: usize, value: u8) -> MorphControllers {
    let mut c = MorphControllers::default();
    let x = f32::from(value) / 127.0;
    match source {
        0 => c.mod_wheel = x,
        1 => c.aftertouch = x,
        _ => c.control_pedal = x,
    }
    c.firmware_weighted()
}

#[test]
fn firmware_depth_sum_rounds_to_the_dispatched_value_the_handler_uses() {
    for (source, value, dispatched, _) in ARM {
        let mut fx = chain();
        fx.apply_morph(&controllers(source, value));
        let want = (dispatched + 0.5).floor() as u16;
        assert_eq!(
            fx.delay_rate_raw,
            Some(want),
            "source {source} value {value}"
        );
    }
}

#[test]
fn morphed_value_gives_the_measured_dsp_time_word() {
    for (source, value, _, word) in ARM {
        let mut fx = chain();
        fx.apply_morph(&controllers(source, value));
        let raw = fx.delay_rate_raw.unwrap();
        assert_eq!(
            fw_delay_time::delay_word(raw),
            word,
            "source {source} value {value}"
        );
    }
}

#[test]
fn all_three_sources_sum_before_the_clamp() {
    // ARM dispatch 11008 for wheel + A.T. + pedal at 127 (1548 + 3096 - 4644 = 0).
    let mut fx = chain();
    let mut c = MorphControllers {
        mod_wheel: 1.0,
        control_pedal: 1.0,
        aftertouch: 1.0,
    };
    c = c.firmware_weighted();
    fx.apply_morph(&c);
    assert_eq!(fx.delay_rate_raw, Some(BASE));
}

#[test]
fn without_depths_or_without_a_raw_value_nothing_moves() {
    let mut fx = LayerFxChain::new(48000.0);
    fx.delay_rate_raw = Some(5000);
    fx.apply_morph(&MorphControllers {
        mod_wheel: 1.0,
        ..Default::default()
    });
    assert_eq!(fx.delay_rate_raw, Some(5000));
    let mut fx = chain();
    fx.delay_rate_raw = None;
    fx.apply_morph(&MorphControllers {
        mod_wheel: 1.0,
        ..Default::default()
    });
    assert_eq!(fx.delay_rate_raw, None);
}

#[test]
fn engine_time_follows_the_morph_only_with_the_firmware_switch() {
    let echo = |switch: bool, wheel: f32| {
        let mut fx = chain();
        fx.firmware_fx.delay_time = switch;
        fx.delay_time_ms = 250.0;
        fx.apply_morph(&MorphControllers {
            mod_wheel: wheel,
            ..Default::default()
        });
        fx.target_delay_samples(120.0)
    };
    assert_eq!(
        echo(false, 0.0),
        echo(false, 1.0),
        "engine time ignores the morph"
    );
    assert!(
        echo(true, 1.0) < echo(true, 0.0),
        "a larger value is a shorter delay"
    );
}
