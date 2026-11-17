//! Vox key-down weight timeline: the ARM sends the weights of one key in bursts, not at once.
//!
//! Research: `docs/research/keydown-zones.md` section 5 and `docs/research/organ-vox-click.md`.
//! The weight queue of the Vox engine (`0xC0224360`) drains the footage slots of a key at fixed
//! delays after the key event; the captured key-down of note 60 sends slot 0 (16') at tick 3,
//! slots 2 and 5 (8', 4') at tick 17 and slot 10 (2') at tick 47, the key-up zeroes slot 10 at
//! tick 1, slots 2 and 5 at tick 7 and slot 0 at tick 37 (`keydown-organ-keys.csv`, model 1, all
//! 24 notes). This module replays that timeline per key; the weights are instantaneous steps at
//! those ticks, which replaces the legacy key click on this source.

use super::vox_keys::{self, ROWS};

/// Seconds per ARM tick, measured: one interrupt of the 24 kHz ARM timer, 1 / 24,000 s = 41.67 us.
///
/// VERIFIED: Timer0 `PRD34 = 1000` at the 24 MHz input clock is 24 kHz, and the slot delays are
/// timer driven (the Vox key-down groups arrive at interrupts 3 / 17 / 47 at every interrupt
/// spacing), so 3 / 17 / 47 ticks are 0.125 / 0.71 / 1.96 ms. The key-down counts are checked by
/// `arm_tick_probe.py keydelay` (`docs/research/data/arm-tick-keydelay.json`) and by
/// `tests::measured_delays_match_the_keydelay_table`; the key-up 1 / 7 / 37 ticks (0.04 / 0.29 /
/// 1.54 ms) come from `docs/research/data/keydown-organ-keys.csv` (model 1, 24 notes, checked by
/// `tests::measured_key_up_follows_the_1_7_37_tick_table`), not from the keydelay json. The 24 MHz board clock is ASSUMED
/// (`docs/research/arm-tick-timers.md` s5); the ARM may need longer than the timer count, so these
/// are lower bounds. Default since D131 (owner approved); `--legacy-1khz-tick` restores
/// [`LEGACY_TICK_SECONDS`].
pub(super) const TICK_SECONDS: f32 = 1.0 / 24_000.0;

/// Legacy tick of the capture script, 1 ms (ASSUMED, no firmware counterpart): the pre-D131
/// timeline, kept behind `--legacy-1khz-tick`.
pub(super) const LEGACY_TICK_SECONDS: f32 = 0.001;

/// Delay in ticks after key-down before slot `slot` is sent.
///
/// VERIFIED for slots 0, 2, 5, 10 (24 notes, `keydown-organ-keys.csv`: 3, 17, 17, 47). ASSUMED
/// for the other slots, which are zero in the default program: slots 0..1 at 3, 2..5 at 17, the
/// rest at 47 (the queue drains in slot order).
pub(super) fn on_delay(slot: usize) -> u32 {
    match slot {
        0..=1 => 3,
        2..=5 => 17,
        _ => 47,
    }
}

/// Delay in ticks after key-up before slot `slot` is zeroed (VERIFIED for slots 0, 2, 5, 10:
/// 37, 7, 7, 1; the other slots follow the same grouping, ASSUMED).
pub(super) fn off_delay(slot: usize) -> u32 {
    match slot {
        0..=1 => 37,
        2..=5 => 7,
        _ => 1,
    }
}

/// Per-key age tracking and the resulting row weights.
#[derive(Debug, Clone)]
pub struct VoxTimeline {
    held: [bool; 128],
    /// Samples since the last key event of the key.
    age: [u32; 128],
    /// Key has weights on or pending (held, or released inside its off delays).
    live: [bool; 128],
    weights: [f32; ROWS],
    countdown: u32,
    /// Seconds per ARM tick ([`TICK_SECONDS`] or, legacy, [`LEGACY_TICK_SECONDS`]).
    tick_seconds: f32,
}

/// Samples between weight recomputations on the legacy tick (0.2 ms at 48 kHz, far below 1 ms).
const LEGACY_REFRESH: u32 = 8;

impl Default for VoxTimeline {
    fn default() -> Self {
        Self {
            held: [false; 128],
            age: [0; 128],
            live: [false; 128],
            weights: [0.0; ROWS],
            countdown: 0,
            tick_seconds: TICK_SECONDS,
        }
    }
}

impl VoxTimeline {
    /// Selects the measured 24 kHz tick (`true`, default) or the legacy 1 ms tick.
    pub fn set_measured_tick(&mut self, on: bool) {
        self.tick_seconds = if on {
            TICK_SECONDS
        } else {
            LEGACY_TICK_SECONDS
        };
    }

    /// `true` on the measured 24 kHz tick, `false` on the legacy 1 ms tick.
    pub fn measured_tick(&self) -> bool {
        self.tick_seconds < LEGACY_TICK_SECONDS
    }

    /// Seconds per tick in use.
    #[cfg(test)]
    fn tick_seconds(&self) -> f32 {
        self.tick_seconds
    }

    /// Samples between recomputations: one tick (at least 1), 8 on the legacy tick.
    fn refresh_interval(&self, fs: f32) -> u32 {
        if self.tick_seconds >= LEGACY_TICK_SECONDS {
            LEGACY_REFRESH
        } else {
            ((self.tick_seconds * fs).round() as u32).max(1)
        }
    }

    /// Advances one sample for the held keys (`shift` semitones of octave shift) and returns the
    /// current row weights.
    pub fn step(
        &mut self,
        held: &[bool; 128],
        shift: i32,
        levels: &[f32; 20],
        fs: f32,
    ) -> &[f32; ROWS] {
        let mut changed = false;
        for (n, &h) in held.iter().enumerate() {
            if h != self.held[n] {
                self.held[n] = h;
                self.age[n] = 0;
                self.live[n] = true;
                changed = true;
            } else {
                self.age[n] = self.age[n].saturating_add(1);
            }
        }
        if changed || self.countdown == 0 {
            self.refresh(shift, levels, fs);
            self.countdown = self.refresh_interval(fs);
        }
        self.countdown -= 1;
        &self.weights
    }

    fn refresh(&mut self, shift: i32, levels: &[f32; 20], fs: f32) {
        self.weights.fill(0.0);
        let tick = self.tick_seconds * fs;
        // the small bias keeps an exact tick boundary (6 samples = 3 ticks at 48 kHz) from
        // flooring to the tick below through f32 rounding
        let ticks = |age: u32| (age as f32 / tick + 1e-3) as u32;
        for n in 0..128 {
            if !self.live[n] {
                continue;
            }
            let t = ticks(self.age[n]);
            let mut any = false;
            for (slot, &level) in levels.iter().enumerate().filter(|(_, &l)| l != 0.0) {
                let on = if self.held[n] {
                    t >= on_delay(slot)
                } else {
                    t < off_delay(slot)
                };
                any |= self.held[n] || t < off_delay(slot);
                if on {
                    self.weights[vox_keys::slot_row(n as i32 + shift, slot)] += level;
                }
            }
            if !any {
                self.live[n] = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48_000.0;

    fn run(t: &mut VoxTimeline, held: &[bool; 128], samples: usize) -> [f32; ROWS] {
        let levels = vox_keys::default_levels();
        let mut w = [0.0; ROWS];
        for _ in 0..samples {
            w = *t.step(held, 0, &levels, FS);
        }
        w
    }

    #[test]
    fn legacy_key_down_sends_the_footages_at_1_ms_ticks_3_17_17_47() {
        let mut t = VoxTimeline::default();
        t.set_measured_tick(false);
        let mut held = [false; 128];
        held[60] = true;
        let ms = |m: f32| (m * FS / 1000.0) as usize;
        let at = |t: &mut VoxTimeline, extra: usize| run(t, &held, extra);
        let w = at(&mut t, ms(1.0));
        assert_eq!(w.iter().filter(|&&x| x != 0.0).count(), 0);
        let w = at(&mut t, ms(3.3) - ms(1.0));
        assert!(
            w[24] > 0.0 && w[36] == 0.0 && w[60] == 0.0,
            "16' only at 3 ms"
        );
        let w = at(&mut t, ms(17.3) - ms(3.3));
        assert!(
            w[36] > 0.0 && w[48] > 0.0 && w[60] == 0.0,
            "8' and 4' at 17 ms"
        );
        let w = at(&mut t, ms(47.3) - ms(17.3));
        assert!(w[60] > 0.0, "2' at 47 ms");
    }

    #[test]
    fn legacy_key_up_zeroes_in_the_reverse_order_and_the_key_dies() {
        let mut t = VoxTimeline::default();
        t.set_measured_tick(false);
        let mut held = [false; 128];
        held[60] = true;
        run(&mut t, &held, FS as usize / 10);
        held[60] = false;
        let ms = |m: f32| (m * FS / 1000.0) as usize;
        let w = run(&mut t, &held, ms(2.0));
        assert!(w[60] == 0.0 && w[48] > 0.0 && w[24] > 0.0, "2' first");
        let w = run(&mut t, &held, ms(8.0) - ms(2.0));
        assert!(
            w[48] == 0.0 && w[36] == 0.0 && w[24] > 0.0,
            "8' and 4' next"
        );
        let w = run(&mut t, &held, ms(40.0));
        assert!(w.iter().all(|&x| x == 0.0), "16' last");
        assert!(!t.live[60]);
    }

    #[test]
    fn rows_that_fold_together_add() {
        let mut t = VoxTimeline::default();
        let mut held = [false; 128];
        held[60] = true;
        held[72] = true; // 16' of C5 is the 8' of C4
        let w = run(&mut t, &held, FS as usize / 10);
        assert!((w[36] - 2.0 * vox_keys::SLOT_LEVEL).abs() < 1e-6);
    }

    /// Index (0-based sample after the key event) at which `row` first becomes non-zero.
    fn first_on(t: &mut VoxTimeline, held: &[bool; 128], row: usize) -> usize {
        let levels = vox_keys::default_levels();
        (0..4000)
            .find(|_| t.step(held, 0, &levels, FS)[row] != 0.0)
            .expect("row never sent")
    }

    /// Samples per measured tick at 48 kHz: 48,000 / 24,000.
    const SAMPLES_PER_TICK: usize = 2;

    #[test]
    fn measured_delays_match_the_keydelay_table() {
        // docs/research/data/arm-tick-keydelay.json: groups at timer interrupts 3, 17 and 47
        let mut held = [false; 128];
        held[60] = true;
        for (row, ticks) in [(24usize, 3usize), (36, 17), (48, 17), (60, 47)] {
            let mut t = VoxTimeline::default();
            assert_eq!(t.tick_seconds(), TICK_SECONDS);
            let n = first_on(&mut t, &held, row);
            assert_eq!(n, ticks * SAMPLES_PER_TICK, "row {row}");
        }
        // 0.125 / 0.71 / 1.96 ms of the research table
        for (ticks, ms) in [(3.0f32, 0.125f32), (17.0, 0.708), (47.0, 1.958)] {
            assert!((ticks * TICK_SECONDS * 1000.0 - ms).abs() < 0.003);
        }
    }

    #[test]
    fn measured_key_up_follows_the_1_7_37_tick_table() {
        let levels = vox_keys::default_levels();
        let mut held = [false; 128];
        held[60] = true;
        let mut t = VoxTimeline::default();
        run(&mut t, &held, 4000);
        held[60] = false;
        let mut gone = [None::<usize>; 3];
        for n in 0..400 {
            let w = *t.step(&held, 0, &levels, FS);
            for (i, row) in [60usize, 48, 24].into_iter().enumerate() {
                if gone[i].is_none() && w[row] == 0.0 {
                    gone[i] = Some(n);
                }
            }
        }
        assert_eq!(
            gone,
            [
                Some(SAMPLES_PER_TICK),
                Some(7 * SAMPLES_PER_TICK),
                Some(37 * SAMPLES_PER_TICK)
            ]
        );
    }

    #[test]
    fn legacy_opt_out_restores_the_1_ms_tick() {
        let mut held = [false; 128];
        held[60] = true;
        let mut t = VoxTimeline::default();
        t.set_measured_tick(false);
        assert_eq!(t.tick_seconds(), LEGACY_TICK_SECONDS);
        // 47 ticks of 1 ms = 47 ms = 2,256 samples; refresh grid of 8 samples
        let n = first_on(&mut t, &held, 60);
        assert!((2256..2264).contains(&n), "{n}");
        t.set_measured_tick(true);
        assert_eq!(t.tick_seconds(), TICK_SECONDS);
    }
}
