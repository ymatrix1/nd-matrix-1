//! Firmware gate length of the Arp / Poly steps (`SynthLayer::arp_firmware_gate`, D166; default on
//! since D168).
//!
//! The ARM pattern player ends an `E` step at the second grid point of its step. The pattern
//! objects boot with the grid `0xC02BC1A0` (flag `[obj + 0xC] = 1`), whose points inside one step
//! are `0` and `0x55555 / 0x80000` = 2/3, so an `E` step lasts 2/3 of the step and the plain
//! arpeggio (Pattern off compiles all-`E` words) is staccato. A `q` / `Q` step holds to the next
//! step's first point (full step).

use super::*;

/// Gate fraction of an `E` step and of the plain arpeggio.
///
/// VERIFIED: ARM replay of A:16 Horizon Whl layers A and C, four key sets, gate 100.0 ms of a
/// 150.0 ms step = 0.667 (`docs/research/data/arp-fw-events-a16.json`, `tools/arp_fw_compare.py`,
/// re-run 2026-11-18) and the boot grid points of the pattern compiler
/// (`tools/ns4_arp_player.py gates`, `docs/research/arm-tick-timers.md` s6). Not seen on
/// hardware. D168 ([arp-gate-flag-trace](../../../../docs/research/arp-gate-flag-trace.md),
/// `tools/arm_arp_flag_trace.py`, `tools/arp_flag_census.py`): the flag `[obj + 0xC]` is written
/// only by the constructor (6 calls in boot, 0 after, 0 stores in 41 runs), and 53 ARM-played
/// layers of 30 of the 32 arpeggiator programs give 2/3 of their grid step.
pub(super) const FIRMWARE_E_FRACTION: f32 = 2.0 / 3.0;

/// The `E` gate in eighths of a step (`pattern_presets::gate_eighths(b'E')`).
const E_EIGHTHS: u8 = 4;

/// Gate fraction of a step whose pattern gate is `eighths` (`0` = rest, `8` = full).
///
/// `E` (4 eighths) maps to 2/3 and `q` / `Q` (8) to the full step; the in-between eighths of the
/// GUI gate dial map linearly on both sides, so the dial stays monotonic.
/// ASSUMED: the interpolation for dial values other than `0`, `4`, `8` (the hardware knows only
/// the four gate codes `E q Q .`).
pub(super) fn firmware_gate_fraction(eighths: u8) -> f32 {
    let e = eighths.min(8);
    if e <= E_EIGHTHS {
        f32::from(e) / f32::from(E_EIGHTHS) * FIRMWARE_E_FRACTION
    } else {
        FIRMWARE_E_FRACTION
            + f32::from(e - E_EIGHTHS) / f32::from(8 - E_EIGHTHS) * (1.0 - FIRMWARE_E_FRACTION)
    }
}

impl SynthLayer {
    /// Fraction of the step after which an Arp / Poly note of gate `eighths` ends.
    pub(super) fn arp_gate_fraction(&self, eighths: u8) -> f32 {
        if self.arp_firmware_gate {
            firmware_gate_fraction(eighths)
        } else {
            f32::from(eighths) / 8.0
        }
    }

    /// Plain arpeggio (Pattern off) with the firmware gate: ends the note at 2/3 of the step.
    /// Without the flag the note lasts to the next step, as the engine always did.
    pub(super) fn plain_gate_release(&mut self, step_sec: f32) {
        if self.arp_firmware_gate && self.arp_sample_counter / step_sec >= FIRMWARE_E_FRACTION {
            self.release_arp_notes();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-frame "an arp voice has its gate open" trace of `frames` frames.
    fn gate_trace(synth: &mut SynthLayer, frames: usize) -> Vec<bool> {
        (0..frames)
            .map(|_| {
                synth.render_frame();
                synth.voices.iter().any(|v| v.active && v.gate)
            })
            .collect()
    }

    fn plain_arp(firmware_gate: bool) -> SynthLayer {
        let mut synth = SynthLayer::new(48_000.0);
        synth.arp_firmware_gate = firmware_gate;
        synth.arp_mode = ArpMode::Up;
        synth.arp_rate_bpm = 120.0; // step = 250 ms = 12,000 frames
        synth.note_on(60, 100);
        synth
    }

    #[test]
    fn gate_fraction_maps_e_to_two_thirds_and_q_to_the_full_step() {
        assert_eq!(firmware_gate_fraction(0), 0.0);
        assert!((firmware_gate_fraction(4) - 2.0 / 3.0).abs() < 1e-6);
        assert!((firmware_gate_fraction(8) - 1.0).abs() < 1e-6);
        assert_eq!(firmware_gate_fraction(200), firmware_gate_fraction(8));
        let mut last = -1.0;
        for e in 0..=8 {
            let f = firmware_gate_fraction(e);
            assert!(f > last, "monotonic at {e}");
            last = f;
        }
    }

    #[test]
    fn plain_arpeggio_gate_lasts_two_thirds_of_a_step_with_the_firmware_gate() {
        // ARM replay: gate 100 ms of a 150 ms step. Here 250 ms steps, first note at 250 ms.
        let trace = gate_trace(&mut plain_arp(true), 30_000);
        let on = trace.iter().position(|&g| g).expect("a note sounds");
        let off = on
            + trace[on..]
                .iter()
                .position(|&g| !g)
                .expect("the gate closes");
        assert!(
            (off - on).abs_diff(8_000) <= 2,
            "gate {} frames, want 8000",
            off - on
        );
        let next = off + trace[off..].iter().position(|&g| g).expect("next note");
        assert!(
            (next - on).abs_diff(12_000) <= 2,
            "retrigger after {} frames",
            next - on
        );
    }

    #[test]
    fn legacy_plain_arpeggio_holds_the_note_to_the_next_step() {
        let trace = gate_trace(&mut plain_arp(false), 30_000);
        let on = trace.iter().position(|&g| g).expect("a note sounds");
        assert!(
            trace[on..].iter().all(|&g| g),
            "gate never closes in a step"
        );
    }

    #[test]
    fn a_single_held_key_retriggers_every_step() {
        // ARM A:16 layer A, key 40: note-on every step (40 x 10 in 1.5 s).
        let rises = |trace: &[bool]| trace.windows(2).filter(|w| !w[0] && w[1]).count();
        let fw = rises(&gate_trace(&mut plain_arp(true), 60_000));
        let legacy = rises(&gate_trace(&mut plain_arp(false), 60_000));
        assert!(fw >= 4, "firmware rises {fw}");
        assert_eq!(legacy, 1, "the engine gate never retriggers a held note");
    }

    #[test]
    fn pattern_e_step_follows_the_firmware_gate() {
        let mut synth = plain_arp(true);
        synth.perf.pattern_on = true;
        synth.perf.pattern_custom = true;
        synth.perf.custom.gate.fill(4);
        let trace = gate_trace(&mut synth, 30_000);
        let on = trace.iter().position(|&g| g).expect("a note sounds");
        let off = on
            + trace[on..]
                .iter()
                .position(|&g| !g)
                .expect("the gate closes");
        assert!(
            (off - on).abs_diff(8_000) <= 2,
            "E gate {} frames",
            off - on
        );
    }
}
