//! Engine-level tests of the firmware synth velocity law (`fw_velocity.rs`, opt-in).

use super::fw_velocity::{amp_gain, filter_factor, key_byte};
use super::test_util::*;
use super::*;

fn peak(s: &mut SynthLayer, velocity: u8) -> f32 {
    s.note_on(60, velocity);
    let out = render_n(s, 4800);
    s.note_off(60);
    render_n(s, 4800);
    out.iter().map(|f| f[0].abs()).fold(0.0, f32::max)
}

fn layer(firmware: bool, setting: f32) -> SynthLayer {
    let mut s = SynthLayer::legacy(48_000.0);
    s.amp_velocity = setting / 3.0;
    s.sustain_level = 1.0;
    s.firmware_velocity = firmware;
    s
}

#[test]
fn on_by_default_and_off_in_legacy() {
    assert!(SynthLayer::new(48_000.0).firmware_velocity);
    assert!(!SynthLayer::legacy(48_000.0).firmware_velocity);
}

#[test]
fn level_ratio_between_velocities_follows_the_amp_gain_table() {
    for setting in [1.0f32, 2.0, 3.0] {
        let (lo, hi) = (40u8, 110u8);
        let a = peak(&mut layer(true, setting), lo);
        let b = peak(&mut layer(true, setting), hi);
        let want = amp_gain(setting as u8, key_byte(f32::from(hi) / 127.0))
            / amp_gain(setting as u8, key_byte(f32::from(lo) / 127.0));
        assert!(
            (b / a / want - 1.0).abs() < 0.02,
            "setting {setting}: got {} want {want}",
            b / a
        );
        assert!(b > a, "louder at the higher velocity");
    }
}

#[test]
fn setting_zero_ignores_velocity_and_flag_off_keeps_the_old_law() {
    let a = peak(&mut layer(true, 0.0), 30);
    let b = peak(&mut layer(true, 0.0), 120);
    assert!((a / b - 1.0).abs() < 1e-4, "setting 0 is flat: {a} {b}");
    let old_lo = peak(&mut layer(false, 3.0), 30);
    let new_lo = peak(&mut layer(true, 3.0), 30);
    assert!(
        (old_lo - new_lo).abs() > 1e-4,
        "the opt-in changes the sound"
    );
}

#[test]
fn filter_envelope_depth_follows_the_key_byte_factor() {
    assert_eq!(filter_factor(key_byte(1.0)), 1.0);
    assert!(filter_factor(key_byte(40.0 / 127.0)) < 0.2);
}
