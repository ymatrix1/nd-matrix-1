//! Tests of the per-port FX selection, the Leslie / Farfisa / sample switches of
//! `FirmwareOptions::apply_exact` (the GUI path) and the summary naming.

use super::*;
use crate::fx::{FirmwareFx, RingAdvance};

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn fx_ports_flag_selects_exactly_the_listed_ports() {
    let o =
        FirmwareOptions::from_args(&args(&["--firmware-fx-ports", "mod1,eq,compressor"])).unwrap();
    assert_eq!(o.fx.ports_on(), ["compressor", "mod1", "eq"]);
    let s = o.summary();
    assert!(s.contains("compressor") && s.contains("mod1") && s.contains("eq"));
    assert!(!s.contains("fx "), "not every port: {s}");
    assert!(FirmwareOptions::from_args(&args(&["--firmware-fx-ports", "x"])).is_err());
    let all = FirmwareOptions::from_args(&args(&["--firmware-fx"])).unwrap();
    assert_eq!(all.fx, FirmwareFx::ALL);
    assert!(all.summary().contains("fx"));
}

#[test]
fn apply_exact_sets_and_clears_individual_fx_ports() {
    let mut e = Stage4Engine::new(48_000.0);
    let mut o = FirmwareOptions::default();
    o.fx.mod2 = true;
    o.fx.reverb = true;
    o.apply_exact(&mut e);
    assert!(e
        .fx_chains
        .iter()
        .all(|c| c.firmware_fx.mod2 && c.firmware_fx.reverb && !c.firmware_fx.eq));
    o.fx.mod2 = false;
    o.apply_exact(&mut e);
    assert!(e
        .fx_chains
        .iter()
        .all(|c| !c.firmware_fx.mod2 && c.firmware_fx.reverb));
    o.apply_to(&mut e);
    assert!(e.fx_chains.iter().all(|c| c.firmware_fx.reverb));
}

#[test]
fn apply_exact_switches_the_firmware_leslie_on_and_off_and_keeps_its_state() {
    let mut e = Stage4Engine::new(48_000.0);
    let mut o = FirmwareOptions {
        leslie_firmware: true,
        leslie_ring: RingAdvance::PASS_RESET8,
        leslie_gain_mdb: 6_000,
        ..Default::default()
    };
    o.apply_exact(&mut e);
    assert_eq!(
        e.organ_a.leslie_fw.as_ref().map(|l| l.ring()),
        Some(RingAdvance::PASS_RESET8)
    );
    assert!((e.organ_b.leslie_gain - 1.9953).abs() < 1e-3);
    let p = e.organ_a.leslie_fw.as_deref().unwrap() as *const _;
    o.apply_exact(&mut e);
    assert_eq!(
        e.organ_a.leslie_fw.as_deref().unwrap() as *const _,
        p,
        "same options keep the processor (and its rotor state)"
    );
    o.leslie_ring = RingAdvance::FROZEN;
    o.apply_exact(&mut e);
    assert_eq!(
        e.organ_a.leslie_fw.as_ref().map(|l| l.ring()),
        Some(RingAdvance::FROZEN)
    );
    assert!(o
        .summary()
        .contains("leslie=firmware ring=frozen gain=+6.0dB"));
    FirmwareOptions::default().apply_exact(&mut e);
    assert!(e.organ_a.leslie_fw.is_none() && e.organ_b.leslie_fw.is_none());
    assert_eq!(e.organ_a.leslie_gain, 1.0);
}

#[test]
fn apply_exact_round_trips_farfisa_sample_and_piano_law_switches() {
    let mut e = Stage4Engine::new(48_000.0);
    let o = FirmwareOptions {
        farfisa_processor: Some(true),
        piano_layer_law: true,
        sample_kernel_key: false,
        sample_pitch_fw: true,
        ..Default::default()
    };
    o.apply_exact(&mut e);
    assert!(e.organ_a.farfisa_processor && e.organ_b.farfisa_processor);
    assert!(e.piano_a.firmware_layer_law && e.piano_b.firmware_layer_law);
    assert!(!e.synth_c.sample_kernel_by_key && e.synth_a.sample_pitch_fw);
    FirmwareOptions {
        farfisa_processor: Some(false),
        ..Default::default()
    }
    .apply_exact(&mut e);
    assert!(!e.organ_a.farfisa_processor && !e.piano_a.firmware_layer_law);
    assert!(e.synth_c.sample_kernel_by_key && !e.synth_a.sample_pitch_fw);
}

#[test]
fn leslie_drive_parses_and_reaches_both_organ_layers() {
    let args = |v: &str| {
        vec![
            "--leslie".into(),
            "firmware".into(),
            "--leslie-drive".into(),
            v.into(),
        ]
    };
    assert!(
        FirmwareOptions::from_args(&args("program"))
            .unwrap()
            .leslie_drive
    );
    assert!(
        !FirmwareOptions::from_args(&args("boot"))
            .unwrap()
            .leslie_drive
    );
    assert!(FirmwareOptions::from_args(&args("x")).is_err());
    let mut e = Stage4Engine::new(48_000.0);
    let o = FirmwareOptions::from_args(&args("program")).unwrap();
    o.apply_exact(&mut e);
    assert!(e.organ_a.leslie_fw_drive && e.organ_b.leslie_fw_drive);
    assert!(o.summary().contains("drive=program"));
    let mut e2 = Stage4Engine::new(48_000.0);
    o.apply_to(&mut e2);
    assert!(e2.organ_a.leslie_fw_drive);
}

#[test]
fn leslie_ring_names_invert_the_parser() {
    for n in ["firmware", "advance", "frozen"] {
        assert_eq!(leslie_ring_name(parse_leslie_ring(n).unwrap()), n);
    }
    assert_eq!(
        leslie_ring_name(RingAdvance {
            stride: 1,
            period: 3
        }),
        "custom"
    );
}

#[test]
fn apply_exact_switches_the_b3_bass_processor_on_and_off() {
    let mut e = Stage4Engine::new(48_000.0);
    let on = FirmwareOptions {
        b3_bass_processor: Some(true),
        ..Default::default()
    };
    on.apply_exact(&mut e);
    assert!(e.organ_a.b3_bass_processor_on() && e.organ_b.b3_bass_processor_on());
    on.apply_exact(&mut e);
    assert!(e.organ_a.b3_bass_processor_on(), "repeat keeps it on");
    FirmwareOptions::default().apply_exact(&mut e);
    assert!(!e.organ_a.b3_bass_processor_on() && !e.organ_b.b3_bass_processor_on());
}

#[test]
fn fx_defaults_flag_is_opt_in_and_sets_the_firmware_words() {
    assert!(
        !FirmwareOptions::default().fx_defaults,
        "D101: off by default"
    );
    let o = FirmwareOptions::from_args(&args(&["--fx-defaults-firmware"])).unwrap();
    assert!(o.fx_defaults && o.summary().contains("fx-defaults"));
    let mut e = Stage4Engine::new(48_000.0);
    let before = e.fx_chains[0].delay_time_ms;
    o.apply_to(&mut e);
    assert_ne!(before, 250.0);
    assert!(e.fx_chains.iter().all(|c| c.delay_time_ms == 250.0));
    let mut plain = Stage4Engine::new(48_000.0);
    FirmwareOptions::default().apply_to(&mut plain);
    assert_eq!(plain.fx_chains[0].delay_time_ms, before);
}

#[test]
fn the_firmware_pump_is_the_default_and_legacy_pump_restores_the_engine_model() {
    // D134: on by default on every chain, `--legacy-pump` turns it off (both apply paths), also
    // together with `--firmware-fx`; it shows in the summary.
    let mut e = Stage4Engine::new(48_000.0);
    assert!(e.fx_chains.iter().all(|c| c.firmware_fx.pump));
    let o = FirmwareOptions::from_args(&args(&["--legacy-pump"])).unwrap();
    assert!(o.legacy_pump && o.summary().contains("legacy-pump"));
    o.apply_to(&mut e);
    assert!(e.fx_chains.iter().all(|c| !c.firmware_fx.pump));
    let mut g = Stage4Engine::new(48_000.0);
    o.apply_exact(&mut g);
    assert!(g
        .fx_chains
        .iter()
        .all(|c| !c.firmware_fx.pump && c.firmware_fx.amp));
    let both = FirmwareOptions::from_args(&args(&["--firmware-fx", "--legacy-pump"])).unwrap();
    let mut h = Stage4Engine::new(48_000.0);
    both.apply_to(&mut h);
    assert!(h
        .fx_chains
        .iter()
        .all(|c| !c.firmware_fx.pump && c.firmware_fx.mod1));
    assert!(!FirmwareOptions::from_args(&args(&[])).unwrap().legacy_pump);
}
