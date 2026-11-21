//! `Stage4Engine::firmware_morph` (D159, default on since D166): ARM controller weights and the
//! extra synth slots; `--legacy-morph` keeps the pre-D166 `v / 127` law.

use super::*;

fn wheel(e: &mut Stage4Engine, value: u8) {
    e.handle_midi(MidiEvent::ControlChange {
        channel: 0,
        controller: cc::MOD_WHEEL,
        value,
    });
    e.render_frame();
}

#[test]
fn legacy_morph_keeps_v_over_127_and_ignores_the_extra_slots() {
    let mut e = Stage4Engine::new(48_000.0);
    e.firmware_morph = false;
    e.synth_morph[0].env_amount.base = 0.25;
    e.synth_morph[0].env_amount.delta_wheel = 0.5;
    e.synth_cutoff_morph[0].base = 0.4;
    e.synth_cutoff_morph[0].delta_wheel = 0.5;
    wheel(&mut e, 64);
    assert_ne!(
        e.synth_a.filter_env_amount, 0.5,
        "slots are off with --legacy-morph"
    );
    let norm = 64.0 / 127.0;
    let want = crate::cbin::cutoff_norm_to_hz(0.4 + 0.5 * norm);
    assert!((e.synth_a.cutoff_hz - want).abs() < 1e-2 * want);
}

#[test]
fn the_firmware_law_is_the_engine_default() {
    assert!(Stage4Engine::new(48_000.0).firmware_morph);
}

#[test]
fn firmware_law_weighs_v_over_128_and_plays_the_env_amount_slot() {
    let mut e = Stage4Engine::new(48_000.0);
    assert!(e.firmware_morph);
    e.synth_morph[1].env_amount.base = 32.0 / 127.0;
    e.synth_morph[1].env_amount.delta_wheel = 23.0 / 127.0;
    wheel(&mut e, 64);
    // ARM run (morph-fw-horizon.csv, Horizon Whl layer B 0x138): 43.5 at wheel 64.
    assert!((e.synth_b.filter_env_amount * 127.0 - 43.5).abs() < 1e-3);
    wheel(&mut e, 127);
    assert!((e.synth_b.filter_env_amount * 127.0 - 55.0).abs() < 1e-3);
    wheel(&mut e, 0);
    assert!((e.synth_b.filter_env_amount * 127.0 - 32.0).abs() < 1e-3);
}

#[test]
fn firmware_law_moves_the_cutoff_by_the_arm_steps() {
    let mut e = Stage4Engine::new(48_000.0);
    e.firmware_morph = true;
    e.synth_cutoff_morph[0].base = 52.0 / 127.0;
    e.synth_cutoff_morph[0].delta_wheel = 10.0 / 127.0;
    wheel(&mut e, 127);
    // ARM run: knob 62 at wheel 127 (Horizon Whl layer A), 494 Hz in the cutoff table.
    assert!(
        (e.synth_a.cutoff_hz - 494.0).abs() < 0.5,
        "{}",
        e.synth_a.cutoff_hz
    );
    wheel(&mut e, 0);
    assert!((e.synth_a.cutoff_hz - 277.0).abs() < 0.5);
}

#[test]
fn firmware_cutoff_sender_drops_a_step_at_half_a_knob() {
    let mut e = Stage4Engine::new(48_000.0);
    e.firmware_morph = true;
    e.synth_cutoff_morph[0].base = 52.0 / 127.0;
    e.synth_cutoff_morph[0].delta_wheel = 10.0 / 127.0;
    wheel(&mut e, 32);
    // ARM run: Q16 54.5 is sent as word 0x01800000 = knob 53.5 (morph-fw-horizon.csv).
    let want = crate::cbin::cutoff_norm_to_hz(53.5 / 127.0);
    assert!(
        (e.synth_a.cutoff_hz - want).abs() < 1e-2 * want,
        "{}",
        e.synth_a.cutoff_hz
    );
    e.firmware_morph = false;
    wheel(&mut e, 32);
    let native = crate::cbin::cutoff_norm_to_hz(52.0 / 127.0 + 10.0 / 127.0 * 32.0 / 127.0);
    assert!((e.synth_a.cutoff_hz - native).abs() < 1e-2 * native);
}

#[test]
fn layer_init_clears_the_extra_slots() {
    let mut e = Stage4Engine::new(48_000.0);
    e.synth_morph[2].resonance.delta_wheel = 0.4;
    e.layer_init(LayerInitTarget::All);
    assert!(!e.synth_morph[2].is_active());
}

#[test]
fn fx_delay_rate_morph_is_played_clipped_and_copied() {
    // ARM run (morph-fw-delay.csv): base 11008, wheel depth +1548 steps, dispatch 11782 at wheel 64.
    let mut e = Stage4Engine::new(48_000.0);
    e.fx_chains[3].delay_rate_raw = Some(11_008);
    let m = &mut e.fx_chains[3].morph.delay_rate;
    m.base = 11_008.0 / 16_383.0;
    m.delta_wheel = 1_548.0 / 16_383.0;
    wheel(&mut e, 64);
    assert_eq!(e.fx_chains[3].delay_rate_raw, Some(11_782));
    wheel(&mut e, 127);
    assert_eq!(e.fx_chains[3].delay_rate_raw, Some(12_556));
    let clip = e.capture_morph(MorphSource::ModWheel);
    e.apply_morph_clip(&clip, MorphSource::Aftertouch);
    assert!((e.fx_chains[3].morph.delay_rate.delta_aftertouch - 1_548.0 / 16_383.0).abs() < 1e-6);
    e.clear_morph(MorphSource::ModWheel);
    assert_eq!(e.fx_chains[3].morph.delay_rate.delta_wheel, 0.0);
}

#[test]
fn osc_env_amount_morph_plays_only_with_the_opt_in_flag_and_the_firmware_law() {
    // FM Harpies layer A: base 21 steps, wheel +2 (ARM: 23 at wheel 127, D206).
    let set = || {
        let mut s = crate::synth_morph::SynthMorphSet::default();
        s.osc_env_amount.base = (21.0 / 60.0 + 1.0) * 0.5;
        s.osc_env_amount.delta_wheel = 2.0 / 120.0;
        s
    };
    let mut e = Stage4Engine::new(48_000.0);
    assert!(!e.synth_osc_env_morph, "default off");
    e.synth_morph[0] = set();
    wheel(&mut e, 127);
    assert_eq!(
        e.synth_a.osc_env_amount, 0.0,
        "flag off: the field is not touched"
    );
    e.synth_osc_env_morph = true;
    wheel(&mut e, 127);
    assert!((e.synth_a.osc_env_amount * 60.0 - 23.0).abs() < 1e-4);
    wheel(&mut e, 64);
    assert!((e.synth_a.osc_env_amount * 60.0 - 22.0).abs() < 1e-4);
    // --legacy-morph keeps the old engine: nothing is played even with the flag on
    let mut legacy = Stage4Engine::new(48_000.0);
    legacy.firmware_morph = false;
    legacy.synth_osc_env_morph = true;
    legacy.synth_morph[0] = set();
    wheel(&mut legacy, 127);
    assert_eq!(legacy.synth_a.osc_env_amount, 0.0);
}
