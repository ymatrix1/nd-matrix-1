//! Master Clock tick generator for the firmware PUMP (`kFx1Type` 10, 11, routine `0x122D2C`).
//!
//! The routine compares the counter word `c[-0x15]` with its copy `c[-0x14]` (`fl_icomp`, signed
//! integer compare, `fw_pump.rs`): **any change** resets the phase `c[-0x1F]` to `0x80000000`
//! and stores the new value in the copy; the counter's value and rate carry no other meaning.
//! One change per Master Clock division therefore gives one duck per division, and the cycle
//! period is the division period (`60 / bpm * beats` s) whatever the counter step is.
//!
//! VERIFIED (DSP side): reset on any change of `c[-0x15]`, none while it is constant, by the
//! oracle-verified port (`fw_pump::run`, 500/500 against `sharc-fxlive`) and the tests below and
//! in `tests_fw_pump_sync.rs`. ASSUMED: that the hardware's writer advances the counter once per
//! division, and its alignment to the beat grid (the writer is unknown, OQ36); the only evidence
//! is the 2.11 Hz of one A:25 clip at 126 BPM (`docs/research/arm-tick-timers.md` s4).

/// Phase increment per call `c[-32]` that makes one phase cycle (`2^32`) last one division of
/// `period` engine frames at two frames per call: `floor(2^33 / period)`, rounded down so the
/// natural wrap never lands before the tick (no double duck). ASSUMED (the hardware's value under
/// Master Clock is not captured, OQ36); the free-run law is ignored in sync because the same
/// `kFx1RateClkDiv` value is the division index there.
pub(super) fn sync_increment(period: f64) -> u32 {
    let inc = (8_589_934_592.0 / period.max(4.0)).floor();
    inc.clamp(1.0, f64::from(i32::MAX)) as u32
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

/// Counter generator: one new counter value every `period` engine frames.
#[derive(Debug, Clone, Default)]
pub(super) struct MasterClockTick {
    started: bool,
    acc: f64,
    counter: u32,
}

impl MasterClockTick {
    /// Restarts the generator: the next [`Self::advance`] ticks at once (a clock that has just
    /// been enabled starts a duck, like the first beat).
    pub(super) fn restart(&mut self) {
        self.started = false;
        self.acc = 0.0;
    }

    /// Advances by `frames` engine frames with a tick every `period` frames. Returns the new
    /// counter word when a tick happened (at most one per call), else `None`. The counter
    /// increments by 1 and wraps; it is never 0 after the first tick so it differs from the boot
    /// value of the copy word.
    pub(super) fn advance(&mut self, frames: f64, period: f64) -> Option<u32> {
        if !(period.is_finite() && period > 0.0) {
            return None;
        }
        let tick = if self.started {
            self.acc += frames;
            if self.acc >= period {
                self.acc %= period;
                true
            } else {
                false
            }
        } else {
            self.started = true;
            true
        };
        tick.then(|| {
            self.counter = self.counter.wrapping_add(1);
            self.counter
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_call_ticks_then_one_tick_per_period() {
        let mut c = MasterClockTick::default();
        assert_eq!(c.advance(2.0, 100.0), Some(1));
        let ticks = (0..1000).filter_map(|_| c.advance(2.0, 100.0)).count();
        assert_eq!(ticks, 20, "2000 frames at a period of 100 frames");
    }

    #[test]
    fn the_counter_changes_on_every_tick() {
        let mut c = MasterClockTick::default();
        let mut last = 0;
        for _ in 0..50 {
            if let Some(v) = c.advance(2.0, 10.0) {
                assert_ne!(v, last);
                last = v;
            }
        }
        assert!(last >= 9);
    }

    #[test]
    fn a_fractional_period_keeps_its_long_term_rate() {
        let mut c = MasterClockTick::default();
        let period = 22_857.142_857; // 126 BPM at 48 kHz, quarter note
        let n = (0..470_000).filter_map(|_| c.advance(2.0, period)).count();
        // 470,000 calls x 2 frames = 940,000 frames = 41.1 periods, plus the first tick.
        assert_eq!(n, 42);
    }

    #[test]
    fn the_sync_increment_spans_one_division() {
        // 126 BPM quarter note at 48 kHz: 11,428.57 calls of two frames per cycle.
        let inc = f64::from(sync_increment(22_857.142_857));
        let calls = 4_294_967_296.0 / inc;
        assert!((calls - 11_428.57).abs() < 0.5, "{calls}");
        assert!(
            calls * 2.0 >= 22_857.142_857 - 1.0e-6,
            "wrap never before the tick"
        );
        // Half the division is twice the increment; absurd periods stay in range.
        let ratio = f64::from(sync_increment(11_428.571_428)) / inc;
        assert!((ratio - 2.0).abs() < 1.0e-3, "{ratio}");
        assert_eq!(sync_increment(0.0), i32::MAX as u32);
        assert_eq!(sync_increment(f64::MAX), 1);
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

    #[test]
    fn restart_ticks_again_and_a_bad_period_never_ticks() {
        let mut c = MasterClockTick::default();
        assert!(c.advance(2.0, 100.0).is_some());
        assert!(c.advance(2.0, 100.0).is_none());
        c.restart();
        assert!(c.advance(2.0, 100.0).is_some());
        for p in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(c.advance(2.0, p).is_none(), "{p}");
        }
    }
}
