//! Firmware PUMP (`FirmwareFx::mod1`, `kFx1Type` 10, 11) driven by the Master Clock word.

use super::*;

fn pump(ty: u8, beats: Option<f32>, bpm: f32) -> LayerFxChain {
    let mut fx = LayerFxChain::new(48_000.0);
    fx.firmware_fx = FirmwareFx {
        pump: true,
        ..FirmwareFx::OFF
    };
    fx.mod1_type = Mod1Type::Pump;
    fx.mod1_type_raw = Some(ty);
    fx.mod1_rate_raw = Some(73); // A:25 Sidechain Whl II
    fx.mod1_amount = 1.0;
    fx.mod1_sync_beats = beats;
    fx.clock_bpm = bpm;
    fx
}

fn gains(fx: &mut LayerFxChain, n: usize) -> Vec<f32> {
    (0..n)
        .map(|_| fx.process_mod1(1.0, 1.0, 1.0 / 48_000.0)[0])
        .collect()
}

/// Duck rate in Hz over `g` at 48 kHz: the gain falls by more than 0.5 within 600 frames (the
/// duck ramp is about 1,000 frames of the 4 ms law at this rate), then 2,000 frames of dead time. The first 2,000 frames
/// (latency, start phase) are skipped.
fn duck_hz(g: &[f32]) -> f64 {
    let (mut n, mut i) = (0, 2_000);
    while i + 600 < g.len() {
        if g[i] - g[i + 600] > 0.5 {
            n += 1;
            i += 2_000;
        } else {
            i += 1;
        }
    }
    f64::from(n) * 48_000.0 / (g.len() - 2_000) as f64
}

fn hz(ty: u8, beats: Option<f32>, bpm: f32, secs: usize) -> f64 {
    let mut fx = pump(ty, beats, bpm);
    duck_hz(&gains(&mut fx, 48_000 * secs))
}

#[test]
fn at_126_bpm_a_quarter_note_pumps_at_2_1_hz_where_the_free_run_pumps_at_6_1_hz() {
    // Hardware A:25 Sidechain Whl II: 2.11 Hz at 126 BPM (`arm-tick-timers.md` s4); the free-run
    // render measured 6.11 Hz. Tolerance 0.1 Hz over 20 s (one duck = 0.05 Hz of resolution).
    let sync = hz(10, Some(1.0), 126.0, 20);
    assert!((sync - 2.10).abs() < 0.1, "sync {sync}");
    let free = hz(10, None, 126.0, 20);
    assert!((free - 6.108).abs() < 0.1, "free run {free}");
}

#[test]
fn the_divisions_scale_the_cycle_and_the_bpm_scales_with_them() {
    // 17 hardware divisions are `mod1_sync_beats` values 0.0625 .. 2 beats (`DELAY_SYNC_BEATS`).
    for beats in [0.5_f32, 0.25, 1.0, 1.5, 2.0] {
        let got = hz(10, Some(beats), 126.0, 20);
        let want = 126.0 / 60.0 / f64::from(beats);
        assert!(
            (got - want).abs() < want * 0.03 + 0.06,
            "beats {beats}: {got} vs {want}"
        );
    }
    let (slow, fast) = (hz(10, Some(1.0), 90.0, 20), hz(10, Some(1.0), 180.0, 20));
    assert!((fast / slow - 2.0).abs() < 0.1, "{slow} {fast}");
}

#[test]
fn both_pump_types_follow_the_clock_and_a_bpm_change_follows_at_once() {
    for ty in [10, 11] {
        let got = hz(ty, Some(1.0), 120.0, 20);
        assert!((got - 2.0).abs() < 0.1, "type {ty}: {got}");
    }
    let mut fx = pump(10, Some(1.0), 60.0);
    let a = duck_hz(&gains(&mut fx, 48_000 * 10));
    fx.clock_bpm = 240.0;
    let b = duck_hz(&gains(&mut fx, 48_000 * 10));
    assert!((a - 1.0).abs() < 0.15 && (b - 4.0).abs() < 0.3, "{a} {b}");
}

#[test]
fn free_run_is_unchanged_and_leaving_the_sync_returns_to_it() {
    let free = |f: &mut LayerFxChain| gains(f, 20_000);
    let mut a = pump(10, None, 126.0);
    let before = free(&mut a);
    // Same chain, sync switched on and off again for 1 s: the stage is rebuilt-free but must
    // return to the rate law of the knob (6.1 Hz).
    let mut b = pump(10, Some(1.0), 126.0);
    gains(&mut b, 48_000);
    b.mod1_sync_beats = None;
    let after = duck_hz(&gains(&mut b, 48_000 * 10));
    assert!((after - 6.108).abs() < 0.2, "{after}");
    // Free run is deterministic and identical to a chain that never saw a clock.
    let mut c = pump(10, None, 126.0);
    assert_eq!(before, free(&mut c));
    // The Master Clock value does not touch the free run.
    let mut d = pump(10, None, 33.0);
    assert_eq!(before, free(&mut d));
}

#[test]
fn only_pump_follows_the_clock_other_types_and_bad_clocks_stay_on_the_engine() {
    // RM in sync: engine path, no 2-frame latency.
    let mut rm = pump(5, Some(1.0), 126.0);
    rm.mod1_type = Mod1Type::RingMod;
    assert_ne!(gains(&mut rm, 16)[8], 0.0, "engine path has no latency");
    // PUMP with a zero or non-finite clock or division falls back to the engine pump.
    for (beats, bpm) in [(Some(0.0), 126.0), (Some(1.0), 0.0), (Some(1.0), f32::NAN)] {
        let mut fx = pump(10, beats, bpm);
        assert_ne!(gains(&mut fx, 16)[8], 0.0, "{beats:?} {bpm}");
    }
    // PUMP in sync is the firmware stage: 2 frames of latency.
    let mut fx = pump(10, Some(1.0), 126.0);
    let g = gains(&mut fx, 8);
    assert!(g[..2].iter().all(|x| *x == 0.0));
}

#[test]
fn a_type_change_or_a_pause_reuses_the_allocation_of_the_firmware_stage() {
    // Control thread: the program load allocates the stage; audio path: type changes and
    // sync / free-run switches re-initialise the same box (no `Box::new` while rendering).
    let mut fx = pump(10, Some(1.0), 126.0);
    fx.prepare_firmware_mod1();
    let at = |fx: &LayerFxChain| fx.fw_mod1_rest.as_deref().map(|b| b as *const _ as usize);
    let first = at(&fx);
    assert!(first.is_some());
    gains(&mut fx, 100);
    fx.mod1_type = Mod1Type::Wah;
    fx.mod1_type_raw = Some(8);
    gains(&mut fx, 100);
    fx.mod1_type = Mod1Type::Pump;
    fx.mod1_type_raw = Some(10);
    fx.mod1_sync_beats = None;
    gains(&mut fx, 100);
    fx.mod1_type = Mod1Type::AutoPan;
    fx.mod1_type_raw = Some(0);
    gains(&mut fx, 100); // parked, not freed
    fx.mod1_type = Mod1Type::Pump;
    fx.mod1_type_raw = Some(11);
    gains(&mut fx, 100);
    assert_eq!(at(&fx), first, "same allocation throughout");
    assert!(gains(&mut fx, 4_000).iter().any(|g| *g != 0.0));
}

#[test]
fn the_default_switches_run_the_firmware_pump_in_sync_and_legacy_pump_does_not() {
    // D134: `FirmwareFx::DEFAULT` has no `mod1`, only `pump`: PUMP in Master Clock sync is the
    // firmware stage (2 frames of latency, 2.1 Hz at 126 BPM); with `pump` off it is the engine
    // model (no latency) and the other types stay on the engine.
    let mut fx = pump(10, Some(1.0), 126.0);
    fx.firmware_fx = FirmwareFx::DEFAULT;
    let g = gains(&mut fx, 48_000 * 20);
    assert!(g[..2].iter().all(|x| *x == 0.0));
    assert!((duck_hz(&g) - 2.10).abs() < 0.1);
    let mut legacy = pump(10, Some(1.0), 126.0);
    legacy.firmware_fx = FirmwareFx {
        pump: false,
        ..FirmwareFx::DEFAULT
    };
    assert_ne!(gains(&mut legacy, 16)[8], 0.0);
    let mut other = pump(5, Some(1.0), 126.0);
    other.mod1_type = Mod1Type::RingMod;
    other.firmware_fx = FirmwareFx::DEFAULT;
    assert_ne!(
        gains(&mut other, 16)[8],
        0.0,
        "RM follows `mod1`, off by default"
    );
}
