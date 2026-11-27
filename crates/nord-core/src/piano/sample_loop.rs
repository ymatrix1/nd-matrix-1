//! Sustain loop of a note stroke, as the firmware SMP voice record plays it.
//!
//! The SMP state machine (`docs/research/smp-slew.md` s3, VERIFIED runs) wraps the stream counter
//! by the loop length `F-7` once it reaches the loop end `F-8`, and frees a voice only at
//! `F-9` (the end of a non-looping stream). `.npno` strokes carry the loop (`NpnoStroke`):
//! a held note therefore never reaches the stream end, it repeats the last third of its sample
//! until the key-up. Inside the loop region (counter above `F-8 - F-7`) the slew rate word is
//! `F+6` instead of `F+7`, a steeper decay: each repetition is quieter
//! (`docs/research/piano-sample-end.md`, issue #640).
//!
//! The engine keeps its own envelope (the sample's natural decay plays as recorded), so only the
//! in-loop decay `F+6` is applied, from 1.0 at the loop start. ASSUMED: starting from 1.0
//! (the hardware level at loop entry also carries `F+7` of the whole hold, which the default path
//! does not model; what would confirm it: a hardware recording of a 10 s held note).

use crate::organ::FIRMWARE_FRAME_RATE_HZ;

/// One slew update per second interrupt (VERIFIED: `slew_voice::key_up_decay_tau_s`, oracle
/// trajectory of the key-up record, max error 1 LSB).
const UPDATES_PER_SECOND: f64 = FIRMWARE_FRAME_RATE_HZ / 2.0;
const TWO_POW_24: f32 = 16_777_216.0;

/// Wraps a read position by the loop length once it reaches the loop end (`F-8`, `F-7`).
#[inline]
pub fn wrap(pos: f64, loop_start: usize, loop_end: usize) -> f64 {
    let len = loop_end.saturating_sub(loop_start) as f64;
    if len < 1.0 || pos < loop_end as f64 {
        return pos;
    }
    pos - len * ((pos - loop_start as f64) / len).floor()
}

/// `true` once the counter is inside the loop region (`counter > F-8 - F-7`, VERIFIED).
#[inline]
pub fn in_loop(pos: f64, loop_start: usize) -> bool {
    pos > loop_start as f64
}

/// `x = d / 2^24` of the in-loop rate word `F+6 = 1 - x`, `d = 2 * -decay` (ASSUMED law, see
/// `NpnoStroke::loop_decay`); `0` (no decay) for a non-negative field.
#[inline]
pub fn rate_x(decay: i16) -> f32 {
    2.0 * f32::from(decay.min(0).unsigned_abs()) / TWO_POW_24
}

/// Gain factor per output frame at `sample_rate_hz` for a rate word `1 - x` applied once per
/// slew update (`ln(1 - x)` is `-x` to 1e-5 for the verified words).
#[inline]
pub fn frame_factor(x: f32, sample_rate_hz: f32) -> f32 {
    (-f64::from(x) * UPDATES_PER_SECOND / f64::from(sample_rate_hz)).exp() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_keeps_the_position_inside_the_loop() {
        assert_eq!(wrap(100.0, 300, 500), 100.0);
        assert_eq!(wrap(499.5, 300, 500), 499.5);
        assert_eq!(wrap(500.0, 300, 500), 300.0);
        assert!((wrap(501.25, 300, 500) - 301.25).abs() < 1e-9);
        // A step far past the end (several loop lengths) still lands inside.
        let w = wrap(1_234.0, 300, 500);
        assert!((300.0..500.0).contains(&w), "{w}");
        // No loop length: untouched.
        assert_eq!(wrap(900.0, 500, 500), 900.0);
    }

    /// The oracle word `F-8` wraps by `F-7`: counter 130 gives 90 with `F-8 = 100`, `F-7 = 40`
    /// (smp-slew s3, VERIFIED runs).
    #[test]
    fn wrap_equals_the_oracle_edge_case() {
        assert_eq!(wrap(130.0, 100 - 40, 100), 90.0);
    }

    #[test]
    fn loop_region_starts_above_the_loop_start() {
        assert!(!in_loop(300.0, 300));
        assert!(in_loop(300.5, 300));
    }

    /// `F+6 = 0x3F7FFE8E` of the Astoria Grand strokes is `d = 370` (`-185`); the oracle replay
    /// shows `F17 = 0.99997795` in the loop (`tools/smp_stream_end.py replay`).
    #[test]
    fn rate_word_matches_the_oracle() {
        let x = rate_x(-185);
        assert!((1.0 - x - f32::from_bits(0x3F7F_FE8E)).abs() < 1e-9);
        assert_eq!(rate_x(0), 0.0);
        assert_eq!(rate_x(37), 0.0);
    }

    #[test]
    fn loop_decays_about_four_db_per_second() {
        let f = frame_factor(rate_x(-185), 48_000.0);
        let db_per_s = 20.0 * f.log10() * 48_000.0;
        assert!((-4.4..-4.0).contains(&db_per_s), "{db_per_s}");
        assert_eq!(frame_factor(0.0, 48_000.0), 1.0);
    }
}
