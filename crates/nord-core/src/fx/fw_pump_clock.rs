//! Master Clock input of the firmware PUMP (`kFx1Type` 10, 11, routine `0x122D2C`).
//!
//! **How the hardware follows the beat (ARM run + routine, D149).** The routine reads its LFO
//! position through the pointer `c[-29]`: `g = f(imul([c[-29]], c[-28]) + c[-18])`. Free run
//! points `c[-29]` at its own phase word `c[-31]` with `c[-28] = 1`. With `kFx1ClkEnable` = 1 the
//! ARM handler of `kFx1RateClkDiv` rewrites **both**: `c[-29]` becomes the address `0xC0C70` (a
//! word of the FX core 1 pass, copied every pass from `DM(0xC0230)`, the head of the SMP serial
//! frame buffer) and `c[-28]` becomes `N(v)`. The own phase, its increment `c[-32]` and the
//! counter `c[-0x15]` then no longer reach the output: the gain is a function of the clock word
//! times `N`, so the duck repeats `N` times per wrap of the clock word. The word is filled on the
//! DSP side (DMA), not by an ARM message: the ARM never writes it (boot log, harness).
//!
//! VERIFIED: pointer, `N` and the unchanged `c[-18]`, `c[-22]` under MST CLK = ARM run
//! (`docs/research/fx-mod-types.md` s7.2, `tools/fx_pump_clock_probe.py`, output `docs/research/data/fx-pump-clock-probe.txt`); `c[-29]` is the only
//! read of the position (the oracle verified port `fw_pump`, 500/500). ASSUMED: the clock word
//! wraps once per 32 beats (what makes A:25, `N` = 32, one duck per beat, EMPIRICAL 2.11 Hz at
//! 126 BPM against 2.10 Hz) and is 0 at the clock start, so the duck falls on the beat
//! (`imul(0, N) + c[-18]` crosses the sign boundary). The writer of the clock word is open (OQ36).

use super::fw_chorus::CTX_I1;

/// Spare context word `c[0x2A]` that holds the engine's clock word; `c[-29]` points at it under
/// the Master Clock (the hardware points at `0xC0C70`).
pub(super) const CLOCK_WORD: i32 = 0x2A;

/// Address of [`CLOCK_WORD`] in the context of the port (the value of `c[-29]` under sync).
pub(super) const CLOCK_ADDRESS: u32 = CTX_I1 + CLOCK_WORD as u32;

/// Span of the clock word in beats (one wrap of 2^32). ASSUMED (see the module note).
const SPAN_BEATS: f32 = 32.0;

/// Phase multiplier `N` for a division of `beats`: `32 / beats` rounded, at least 1. Equals the
/// ARM's `N(v)` for the stored rates (`sync_beats(v) = 32 / N`).
pub(super) fn multiplier(beats: f32) -> u32 {
    if beats.is_finite() && beats > 0.0 {
        (SPAN_BEATS / beats).round().clamp(1.0, 4096.0) as u32
    } else {
        1
    }
}

/// Clock word generator: a 32 bit saw that wraps once every `span` engine frames.
#[derive(Debug, Clone, Default)]
pub(super) struct ClockWord {
    frac: f64,
}

impl ClockWord {
    /// Restarts at word 0 (the clock start).
    pub(super) fn restart(&mut self) {
        self.frac = 0.0;
    }

    /// Advances by `frames` engine frames over a wrap of `span` frames and returns the word
    /// before the step. A bad span holds the word.
    pub(super) fn advance(&mut self, frames: f64, span: f64) -> u32 {
        let word = (self.frac * 4_294_967_296.0) as u32;
        if span.is_finite() && span > 0.0 {
            self.frac = (self.frac + frames / span).fract();
        }
        word
    }
}

/// First rate value `v` (7 bit `kFx1RateClkDiv`) of each of the 15 steps of the PUMP phase
/// multiplier `c[-28]` the ARM writes under MST CLK, and that multiplier `N`.
///
/// VERIFIED: harness run (`tools/fx_clients.py` unit 0, type 10, `kFx1ClkEnable` 1, `kFx1RateClkDiv`
/// 0..=127, word `c[-28]`, `docs/research/data/fx-pump-sync-n.csv`): 128 of 128 values; without sync
/// `c[-28]` is 1. ASSUMED: that the phase spans 32 beats, so `N` cycles per 32 beats = `N / 32`
/// ducks per beat; it reproduces the one hardware clip (A:25 rate 73: `N` = 32, one beat, 2.11 Hz
/// measured at 126 BPM, EMPIRICAL).
const SYNC_STEPS: [(u32, f32); 15] = [
    (0, 2.0),
    (9, 3.0),
    (18, 4.0),
    (26, 6.0),
    (35, 8.0),
    (43, 12.0),
    (52, 16.0),
    (60, 24.0),
    (69, 32.0),
    (77, 48.0),
    (86, 64.0),
    (94, 96.0),
    (103, 128.0),
    (111, 192.0),
    (120, 256.0),
];

/// Beats per PUMP cycle under MST CLK for the stored 7 bit rate `v`: `32 / N(v)`.
pub(crate) fn sync_beats(v: u32) -> f32 {
    let n = SYNC_STEPS
        .iter()
        .rev()
        .find(|(from, _)| v >= *from)
        .map_or(2.0, |s| s.1);
    32.0 / n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_word_wraps_once_per_span_and_restarts_at_zero() {
        let mut c = ClockWord::default();
        assert_eq!(c.advance(2.0, 1000.0), 0);
        let mut last = 0;
        let mut wraps = 0;
        for _ in 0..999 {
            let w = c.advance(2.0, 1000.0);
            if w < last {
                wraps += 1;
            }
            last = w;
        }
        assert_eq!(wraps, 1, "2000 frames over a 1000 frame span");
        assert!((f64::from(last) / 4_294_967_296.0 - 0.998).abs() < 1.0e-3);
        c.restart();
        assert_eq!(c.advance(2.0, 1000.0), 0);
        let held = c.advance(2.0, 0.0);
        assert_eq!(c.advance(2.0, f64::NAN), held, "a bad span holds the word");
    }

    #[test]
    fn the_multiplier_is_the_arm_n_for_the_stored_rates() {
        assert_eq!(multiplier(1.0), 32);
        assert_eq!(multiplier(0.125), 256);
        assert_eq!(multiplier(16.0), 2);
        assert_eq!(multiplier(0.0), 1);
        assert_eq!(multiplier(f32::NAN), 1);
        for v in 0..128 {
            assert_eq!(
                multiplier(sync_beats(v)) as f32,
                32.0 / sync_beats(v),
                "v {v}"
            );
        }
    }

    #[test]
    fn the_sync_law_matches_the_captured_steps() {
        assert_eq!(sync_beats(73), 1.0, "A:25: one duck per beat");
        assert_eq!(sync_beats(0), 16.0);
        assert_eq!(sync_beats(8), 16.0);
        assert_eq!(sync_beats(9), 32.0 / 3.0);
        assert_eq!(sync_beats(68), 32.0 / 24.0);
        assert_eq!(sync_beats(127), 0.125);
        let csv = include_str!("../../../../docs/research/data/fx-pump-sync-n.csv");
        for line in csv.lines().skip(1) {
            let (v, n) = line.split_once(',').unwrap();
            let (v, n): (u32, f32) = (v.parse().unwrap(), n.parse().unwrap());
            assert_eq!(sync_beats(v), 32.0 / n, "v {v}");
        }
    }
}
