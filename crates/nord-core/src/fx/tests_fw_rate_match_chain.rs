//! `LayerFxChain::set_fx_rate_match`: the firmware Amp stage run at the DSP frame rate
//! (`fw_rate_match`, D137). The reference is the same stage in a chain built at the firmware
//! frame rate, where one host frame is one firmware frame, so its response in Hz is the oracle's
//! (the stage is bit exact per frame, `tests_amp_golden.rs`).

use super::*;
use crate::organ::FIRMWARE_FRAME_RATE_HZ;

fn small_ii(fs: f32) -> LayerFxChain {
    let mut fx = LayerFxChain::new(fs);
    fx.bypass_reverb = true;
    fx.firmware_fx = FirmwareFx::DEFAULT;
    fx.amp_cabinet = AmpCabinet::Small;
    fx.amp_variation = true;
    fx.drive_amount = 0.37;
    fx
}

/// Steady-state amplitude of the left output for a sine of `hz` at amplitude `amp`.
fn tone_gain_db(fx: &mut LayerFxChain, fs: f32, hz: f32, amp: f32) -> f64 {
    let n = 24_000;
    let (mut s, mut c, mut p) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let ph = std::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(fs);
        let x = amp * ph.sin() as f32;
        let y = fx.process_frame([x, x])[0];
        if i > 4000 {
            s += f64::from(y) * ph.sin();
            c += f64::from(y) * ph.cos();
            p += 1.0;
        }
    }
    20.0 * (2.0 * (s * s + c * c).sqrt() / p / f64::from(amp)).log10()
}

#[test]
fn rate_match_is_on_by_default_and_off_restores_the_unmatched_path_bit_for_bit() {
    let drive = |fx: &mut LayerFxChain| -> Vec<[f32; 2]> {
        (0..600)
            .map(|n| {
                let x = (n as f32 * 0.21).sin() * 0.2;
                fx.process_frame([x, -x])
            })
            .collect()
    };
    // D176: a new chain at a supported host rate is matched.
    let mut plain = small_ii(48_000.0);
    assert!(plain.fx_rate_match());
    let matched = drive(&mut plain);
    let mut off = small_ii(48_000.0);
    off.set_fx_rate_match(false);
    assert!(!off.fx_rate_match());
    let reference = drive(&mut off);
    assert_ne!(
        matched, reference,
        "the matched path adds latency and resamples"
    );
    let mut toggled = small_ii(48_000.0);
    toggled.set_fx_rate_match(false);
    toggled.set_fx_rate_match(true);
    toggled.set_fx_rate_match(false);
    assert_eq!(drive(&mut toggled), reference);
    // Below the supported host rates the switch stays off, on or not.
    let mut low = small_ii(22_050.0);
    low.set_fx_rate_match(true);
    assert!(!low.fx_rate_match());
}

#[test]
fn matched_amp_follows_the_frame_rate_response_where_the_unmatched_amp_does_not() {
    let f_rate = FIRMWARE_FRAME_RATE_HZ as f32;
    // tones on the slopes of the Small II cabinet, where the 1.106 stretch moves the gain by
    // 4.1, 4.5 and 8.4 dB (measured: 3, 6 and 7 kHz) and 7.5 dB at 9 kHz
    for hz in [3000.0f32, 6000.0, 7000.0, 9000.0] {
        let want = tone_gain_db(&mut small_ii(f_rate), f_rate, hz, 0.03);
        let mut u = small_ii(48_000.0);
        u.set_fx_rate_match(false);
        let unmatched = tone_gain_db(&mut u, 48_000.0, hz, 0.03);
        let mut m = small_ii(48_000.0);
        m.set_fx_rate_match(true);
        let matched = tone_gain_db(&mut m, 48_000.0, hz, 0.03);
        assert!(
            (matched - want).abs() < 0.3,
            "{hz} Hz: matched {matched} vs frame rate {want}"
        );
        assert!(
            (unmatched - want).abs() > 3.0,
            "{hz} Hz: unmatched {unmatched} vs frame rate {want} should differ"
        );
    }
}

#[test]
fn matched_chain_is_finite_silent_for_silence_and_only_affects_firmware_amps() {
    let mut m = small_ii(48_000.0);
    m.set_fx_rate_match(true);
    for _ in 0..3000 {
        assert_eq!(m.process_frame([0.0, 0.0]), [0.0, 0.0]);
    }
    for n in 0..20_000 {
        let x = (n as f32 * 0.4).sin() * 0.9;
        let [l, r] = m.process_frame([x, x]);
        assert!(l.is_finite() && r.is_finite());
    }
    // an engine-path amp (To Rotary) is untouched by the switch
    let run = |on: bool| {
        let mut fx = LayerFxChain::new(48_000.0);
        fx.bypass_reverb = true;
        fx.amp_cabinet = AmpCabinet::ToRotary;
        fx.set_fx_rate_match(on);
        (0..300)
            .map(|n| fx.process_frame([(n as f32 * 0.1).sin() * 0.3; 2]))
            .collect::<Vec<_>>()
    };
    assert_eq!(run(true), run(false));
}
