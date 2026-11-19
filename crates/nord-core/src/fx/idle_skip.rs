//! Opt-in idle skip of a layer FX rack (`LayerFxChain::set_idle_skip`, `--fx-idle-skip`).
//!
//! The seven racks run every frame, also those of layers that are not sounding (the other scene,
//! an unused layer). With the firmware FX ports on that is most of the CPU: a firmware reverb is an
//! instruction-level model and costs about 1 ms per 480 frames per rack (`ns4 bench --gui`,
//! `docs/research/gui-audio-glitches.md`). The skip stops a rack once its input has been exactly
//! zero and its output below [`QUIET`] for [`QUIET_SECONDS`] in a row, and returns silence until
//! the input is non-zero again; the rack then resumes from its frozen state.
//!
//! Not bit-identical to the always-run path once a skipped rack wakes up (its modulation phases
//! and reverb read pointers stand still while it sleeps), so it is a switch and off by default
//! (D161). A rack that was never fed is silent in both paths, so a render in which every sounding
//! layer keeps playing hashes identically (`bench_gui_prog::tests::idle_skip_keeps_the_hash_of_a_scene`).
//! Allocation free, plain field reads and writes.

/// ASSUMED: output level (linear, -140 dBFS, below one 24-bit step of 1.2e-7) under which a rack
/// counts as quiet. A reverb tail passes below it long after it is inaudible; confirm by ear on a
/// Cathedral tail if the level is ever raised.
pub(super) const QUIET: f32 = 1.0e-7;
/// ASSUMED: quiet time before the rack sleeps. It must exceed the longest silent gap inside a
/// running rack (a delay echo of the longest delay time, two seconds, with feedback), so 4 s.
pub(super) const QUIET_SECONDS: f32 = 4.0;

/// The skip state of one rack.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct IdleSkip {
    /// `Some(frames)` when the switch is on: the quiet run after which the rack sleeps.
    limit: Option<u32>,
    quiet: u32,
}

impl IdleSkip {
    /// Switches the skip on for a rack at `sample_rate`, or off; restarts the quiet count.
    pub(super) fn set(&mut self, on: bool, sample_rate: f32) {
        self.limit = on.then(|| (QUIET_SECONDS * sample_rate).max(1.0) as u32);
        self.quiet = 0;
    }

    pub(super) fn enabled(&self) -> bool {
        self.limit.is_some()
    }

    /// True when the rack sleeps and this frame is silent too.
    #[inline]
    pub(super) fn skips(&self, input: [f32; 2]) -> bool {
        matches!(self.limit, Some(l) if self.quiet >= l) && input == [0.0, 0.0]
    }

    /// Counts the frame the rack just processed.
    #[inline]
    pub(super) fn observe(&mut self, input: [f32; 2], out: [f32; 2]) {
        if self.limit.is_some() {
            let silent = input == [0.0, 0.0] && out[0].abs() < QUIET && out[1].abs() < QUIET;
            self.quiet = if silent {
                self.quiet.saturating_add(1)
            } else {
                0
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::LayerFxChain;

    #[test]
    fn state_machine_sleeps_after_the_quiet_run_and_wakes_on_input() {
        let mut s = IdleSkip::default();
        assert!(!s.enabled() && !s.skips([0.0; 2]));
        s.observe([0.0; 2], [0.0; 2]);
        assert_eq!(s.quiet, 0, "off: nothing is counted");
        s.set(true, 10.0);
        assert_eq!(s.limit, Some(40));
        for _ in 0..39 {
            s.observe([0.0; 2], [0.0; 2]);
        }
        assert!(!s.skips([0.0; 2]));
        s.observe([0.0; 2], [0.0; 2]);
        assert!(s.skips([0.0; 2]) && s.skips([-0.0, 0.0]));
        assert!(!s.skips([0.1, 0.0]), "input wakes the rack");
        assert!(!s.skips([f32::NAN, 0.0]));
        s.observe([0.1, 0.0], [0.1, 0.0]);
        assert_eq!(s.quiet, 0);
        s.observe([0.0; 2], [1.0e-3, 0.0]);
        assert_eq!(s.quiet, 0, "an audible tail keeps the rack running");
        s.set(false, 10.0);
        assert!(!s.skips([0.0; 2]));
    }

    #[test]
    fn a_sleeping_rack_returns_silence_and_resumes_from_its_state() {
        let mut fx = LayerFxChain::new(100.0);
        fx.set_idle_skip(true);
        assert!(fx.idle_skip());
        for n in 0..1000 {
            let x = if n < 5 { 0.5 } else { 0.0 };
            let [l, r] = fx.process_frame([x, x]);
            assert!(l.is_finite() && r.is_finite());
        }
        assert_eq!(fx.process_frame([0.0, 0.0]), [0.0, 0.0], "asleep");
        let [l, r] = fx.process_frame([0.5, 0.5]);
        assert!(l.is_finite() && r.is_finite());
        fx.set_idle_skip(false);
        assert!(!fx.idle_skip());
    }
}
