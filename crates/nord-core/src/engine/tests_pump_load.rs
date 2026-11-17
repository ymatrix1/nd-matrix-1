//! PUMP Master Clock sync through the program load path (`apply_program` -> `apply_fx_chain`):
//! the stored MST CLK flag and the 7 bit rate reach `LayerFxChain::mod1_sync_beats` and the
//! firmware PUMP follows the clock at the BPM of the program.

use super::*;
use crate::fx::FirmwareFx;

/// Program with a PUMP in the Synth A FX chain: `rate_raw` is the stored 7 bit rate.
fn program(rate_raw: u8, sync: bool, enabled: bool, bpm: f32) -> crate::cbin::ProgramDecoded {
    let mut prog = crate::cbin::ProgramDecoded::default();
    prog.global.tempo_bpm = bpm;
    let fx = &mut prog.synth.fx_a;
    fx.mod1_enabled = enabled;
    fx.mod1_clock_sync = sync;
    fx.mod1_type_raw = 10;
    fx.mod1_rate = f32::from(rate_raw) / 127.0;
    fx.mod1_amount = 1.0;
    prog
}

/// Chain that took the Synth A FX of the program.
fn loaded(prog: &crate::cbin::ProgramDecoded) -> LayerFxChain {
    let mut engine = Stage4Engine::new(48_000.0);
    engine.apply_program(prog);
    let mut fx = engine
        .fx_chains
        .iter()
        .find(|c| c.mod1_type_raw == Some(10))
        .expect("the PUMP chain")
        .clone();
    fx.firmware_fx = FirmwareFx::ALL;
    fx
}

/// Ducks per second of the PUMP gain (falls by more than 0.5 within 600 frames).
fn duck_hz(fx: &mut LayerFxChain, bpm: f32, secs: usize) -> f64 {
    let g: Vec<f32> = (0..48_000 * secs)
        .map(|_| fx.process_frame_with_bpm([1.0, 1.0], bpm)[0])
        .collect();
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

#[test]
fn a_synced_pump_program_follows_the_clock_at_the_program_bpm() {
    // Rate 73 (A:25) is the step with N = 32: one duck per beat, 126 BPM -> 2.10 Hz (hardware 2.11 Hz).
    let mut fx = loaded(&program(73, true, true, 126.0));
    assert_eq!(fx.mod1_sync_beats, Some(1.0));
    let hz = duck_hz(&mut fx, 126.0, 20);
    assert!((hz - 2.10).abs() < 0.1, "{hz}");
}

#[test]
fn without_mst_clk_or_when_disabled_the_pump_free_runs() {
    for (sync, enabled) in [(false, true), (true, false)] {
        let fx = loaded(&program(73, sync, enabled, 126.0));
        assert_eq!(fx.mod1_sync_beats, None, "sync {sync} enabled {enabled}");
    }
    let mut free = loaded(&program(73, false, true, 126.0));
    let hz = duck_hz(&mut free, 126.0, 20);
    assert!((hz - 6.108).abs() < 0.2, "free run at rate 73: {hz}");
}

#[test]
fn only_the_pump_types_load_the_clock() {
    let mut prog = program(73, true, true, 126.0);
    prog.synth.fx_a.mod1_type_raw = 0; // A-PAN keeps its free running engine model
    let mut engine = Stage4Engine::new(48_000.0);
    engine.apply_program(&prog);
    assert!(engine.fx_chains.iter().all(|c| c.mod1_sync_beats.is_none()));
}

#[test]
fn the_stored_rate_selects_the_captured_division() {
    // 15 steps of the ARM's phase multiplier N (`fx-pump-sync-n.csv`): beats = 32 / N.
    for (rate, beats) in [
        (0, 16.0),
        (35, 4.0),
        (73, 1.0),
        (100, 1.0 / 3.0),
        (127, 0.125),
    ] {
        let fx = loaded(&program(rate, true, true, 126.0));
        assert_eq!(fx.mod1_sync_beats, Some(beats), "rate {rate}");
    }
}
