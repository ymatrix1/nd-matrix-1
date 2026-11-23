//! Envelope, velocity, filter-bypass and keyboard-tracking tests of the synth layer.

use super::test_util::*;
use super::*;

#[test]
fn dedicated_filter_envelope_uses_adr_and_velocity() {
    let mut s = SynthLayer::legacy(48_000.0);
    s.filter_env_dedicated = true;
    s.filter_env_amount = 1.0;
    s.filter_env_attack_sec = 0.001;
    s.filter_env_decay_sec = 0.01;
    s.filter_env_vel_amount = 1.0;
    s.note_on(60, 127);
    render_n(&mut s, 4800);
    let hi = s.voices.iter().find(|v| v.active).unwrap();
    assert!(hi.fenv_attack_done);
    assert!(
        hi.fenv < 0.05,
        "decay should fall toward 0, got {}",
        hi.fenv
    );
    s.note_off(60);
    let before = s.voices.iter().find(|v| v.active).unwrap().fenv;
    render_n(&mut s, 100);
    let after = s.voices.iter().find(|v| v.active).unwrap().fenv;
    assert!(after <= before);
}

#[test]
fn filter_envelope_decay_follows_the_measured_time_constant() {
    // Laser-zap clip: displayed Decay 2.07 s, audible tau 0.38 s (0.18 x display).
    let mut s = SynthLayer::legacy(48_000.0);
    s.filter_env_dedicated = true;
    s.filter_env_amount = 0.74;
    s.filter_env_attack_sec = 0.0005;
    s.filter_env_decay_sec = 2.07;
    s.note_on(67, 127);
    render_n(&mut s, 48_000 / 2);
    let fenv = s.voices.iter().find(|v| v.active).unwrap().fenv;
    let want = (-(0.5 - 0.0005) / (2.07 * ENV_DECAY_TAU_FRACTION)).exp();
    assert!((fenv - want).abs() < 0.03, "fenv {fenv} want {want}");
    // 7.4 on the panel is a ~5.9 octave sweep at its start.
    let octaves = 0.74 * FILTER_ENV_OCTAVES;
    assert!((5.0..7.0).contains(&octaves));
}

#[test]
fn amp_velocity_zero_ignores_velocity_and_one_follows_it() {
    let run = |sens: f32, vel: u8| {
        let mut s = SynthLayer::legacy(48_000.0);
        s.amp_velocity = sens;
        s.note_on(60, vel);
        energy(&render_n(&mut s, 4000))
    };
    assert!((run(0.0, 20) - run(0.0, 127)).abs() < 1e-3);
    assert!(run(1.0, 20) < 0.2 * run(1.0, 127));
}

#[test]
fn filter_off_bypasses_a_closed_filter() {
    let run = |on: bool| {
        let mut s = SynthLayer::legacy(48_000.0);
        s.cutoff_hz = 40.0;
        s.filter_on = on;
        s.note_on(81, 100);
        energy(&render_n(&mut s, 4000))
    };
    assert!(run(false) > 10.0 * run(true));
}

#[test]
fn oscillator_envelope_modulates_pitch_or_osc_ctrl_only_when_amount_set() {
    let run = |amount: f32, to_pitch: bool| {
        let mut s = SynthLayer::legacy(48_000.0);
        s.osc_mode = SynthOscMode::Wavetable;
        s.osc_env_amount = amount;
        s.osc_env_to_pitch = to_pitch;
        s.osc_env_attack_sec = 0.001;
        s.osc_env_decay_sec = 0.2;
        s.note_on(60, 100);
        render_n(&mut s, 3000)
    };
    assert_eq!(run(0.0, false), run(0.0, true));
    assert_ne!(run(1.0, true), run(0.0, true));
    assert_ne!(run(1.0, false), run(0.0, false));
    assert!(run(-1.0, true).iter().all(|f| f[0].is_finite()));
}

#[test]
fn osc_envelope_decay_uses_the_same_time_constant_fraction() {
    // Pitch-blip clip: displayed Decay 100 ms, audible tau about 18 ms.
    let mut s = SynthLayer::legacy(48_000.0);
    s.osc_env_to_pitch = true;
    s.osc_env_amount = 1.0;
    s.osc_env_attack_sec = 0.0005;
    s.osc_env_decay_sec = 0.1;
    s.note_on(60, 100);
    render_n(&mut s, 480); // attack done (~10 ms)
    let at_start = s.voices.iter().find(|v| v.active).unwrap().oenv;
    render_n(&mut s, 864); // 18 ms more
    let after_18ms = s.voices.iter().find(|v| v.active).unwrap().oenv;
    let ratio = after_18ms / at_start;
    let want = (-0.018f32 / (0.1 * ENV_DECAY_TAU_FRACTION)).exp();
    assert!((ratio - want).abs() < 0.03, "ratio {ratio} want {want}");
}

#[test]
fn test_synth_kb_track_and_14hz_floor() {
    let mut s_track = SynthLayer::legacy(48_000.0);
    s_track.cutoff_hz = 220.0;
    s_track.kb_track = 3; // 1/1 octave tracking
    s_track.note_on(84, 120); // +2 octaves above C4 -> 4x cutoff -> brighter than low note
    let mut rms_high = 0.0f32;
    for _ in 0..512 {
        let [l, _] = s_track.render_frame();
        rms_high += l * l;
    }

    let mut s_notrack = SynthLayer::legacy(48_000.0);
    s_notrack.cutoff_hz = 220.0;
    s_notrack.kb_track = 0;
    s_notrack.note_on(84, 120);
    let mut rms_flat = 0.0f32;
    for _ in 0..512 {
        let [l, _] = s_notrack.render_frame();
        rms_flat += l * l;
    }
    assert!(
        rms_high > rms_flat * 1.5,
        "1/1 kb_track must open filter on C6 ({rms_high} vs {rms_flat})"
    );

    let mut flt = ZdfFilter::default();
    let out = flt.process(1.0, FilterModel::Lp24, 14.0, 0.0, 0.0, 48_000.0);
    assert!(out.is_finite());
}

#[test]
fn fw_pitch_decay_stops_at_half_the_peak_like_the_oracle_chain() {
    let run = |fw: bool| {
        let mut s = SynthLayer::legacy(48_000.0);
        s.osc_env_to_pitch = true;
        s.osc_env_amount = 0.5;
        s.osc_env_fw_pitch = fw;
        s.osc_env_attack_sec = 0.001;
        s.osc_env_decay_sec = 0.05;
        s.note_on(60, 100);
        for _ in 0..48_000 {
            s.render_frame();
        }
        s.voices.iter().find(|v| v.active).unwrap().oenv
    };
    assert!((run(true) - 0.5).abs() < 1e-4, "{}", run(true));
    assert!(run(false) < 0.01, "default decays to 0");
}

/// Settled pitch factor of a voice with `osc_env_velocity` on: `v.oenv` is 1.0 (no decay), so the
/// excursion is `48 * word * G / 128` semitones.
fn fw_pitch_vel_semitones(velocity: u8) -> f32 {
    let mut s = SynthLayer::legacy(48_000.0);
    s.osc_env_to_pitch = true;
    s.osc_env_velocity = true;
    s.osc_env_amount = 0.5;
    s.osc_env_fw_pitch = true;
    s.osc_env_attack_sec = 0.0005;
    s.osc_env_decay_sec = 60.0;
    s.note_on(60, velocity);
    for _ in 0..4800 {
        s.render_frame();
    }
    let v = s.voices.iter().find(|v| v.active).unwrap();
    let g =
        crate::synth::fw_velocity::filter_factor(crate::synth::fw_velocity::key_byte(v.velocity));
    crate::synth_osc_env_word::osc_env_pitch_semitones(0.5 * 60.0, v.oenv * g)
}

#[test]
fn fw_pitch_velocity_scales_by_g_over_128_like_the_oracle() {
    // VERIFIED (D222): oracle Wooden Queen, key byte g 0 / 16 / 32 / 64 / 98 / 127 gives
    // 0 / 2.76 / 5.52 / 11.03 / 16.9 / 22.07 st = `22.07 G / 128` (G = 128 at g = 127).
    let top = fw_pitch_vel_semitones(127);
    assert!(top > 1.0);
    let g = |vel: u8| {
        crate::synth::fw_velocity::effective_g(crate::synth::key_velocity::key_velocity_g(vel))
            as f32
    };
    for vel in [1u8, 40, 64, 100] {
        let want = top * g(vel) / 128.0;
        assert!(
            (fw_pitch_vel_semitones(vel) - want).abs() < 0.02 * top,
            "vel {vel}"
        );
    }
    assert!(fw_pitch_vel_semitones(1) < 0.01 * top, "g = 0 is silent");
}
