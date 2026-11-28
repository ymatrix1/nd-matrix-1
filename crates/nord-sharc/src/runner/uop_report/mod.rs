//! Deterministic counter report of the uop engine: the JSON `tools/uop_gates.py` reads, a one-line
//! summary for people, and the cost model that turns counters into modeled host cycles (design
//! sections 9 and 12).
//!
//! OWNER: W1-RUN. Every number is a count, never a time, so `tools/uop_gates.py` can evaluate the
//! structural gates (G2) and the model gates (K3) without a benchmark. The JSON field names are
//! stable (schema [`SCHEMA`], pinned by `the_json_of_schema_1_is_pinned_byte_for_byte` in
//! [`json`] and by `test_the_rust_golden_report_validates` in `tools/tests/test_uop_gates.py`):
//! [`UopReport::to_json`] is hand-written (this crate has no dependencies) and `nord-tools`'
//! `uop-report` and `lle_sharc_bench` print it. The costs are in [`model`], the single-thread
//! measuring loop that produces steady-state counters in [`measure`], the block-length histogram
//! in [`hist`].
//!
//! A report says which passes its counters cover ([`Window`]): the steady-state gates
//! (`maintain_calls == 0`, `smc == 0`) are only meaningful on the measured passes, so a report
//! that includes the warm-up is marked `all` and the gate evaluator refuses to read them from it.
//! The checked engine's replay counts ([`Replay`]) are report fields of their own: they count
//! passes, while `stats.shadow_checks` keeps the engine's meaning (blocks checked).
pub mod hist;
mod json;
pub mod measure;
pub mod model;
mod summary;

pub use hist::BlockHistogram;
pub use model::{
    model_cycles, Tier, BUDGET_CYCLES, DISPATCH, INTERP_STEP, NATIVE_PER_STEP, TRANSITION,
    WORK_PER_STEP,
};
pub use summary::{build_summary, counters_text};

use crate::core::{Core, UopStats};

/// Schema version of the JSON; bump when a field is renamed or removed (a field added is not a
/// new schema: the gate evaluator ignores what it does not know).
pub const SCHEMA: u32 = 1;

/// Which passes the counters of a report cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    /// The measured passes only: the warm-up passes were reset away or subtracted. What the
    /// steady-state gates need.
    Measured,
    /// Every pass since the engine was armed, warm-up included: a first-pass `maintain_calls` or
    /// `smc` is in it, so the steady-state gates cannot be read from it.
    All,
}

impl Window {
    /// The name in the JSON: `measured` or `all`.
    pub fn name(self) -> &'static str {
        match self {
            Window::Measured => "measured",
            Window::All => "all",
        }
    }
}

/// Passes the `uop-checked` engine replayed on the interpreter, among the measured ones. Pass
/// counts, not the engine's block counts (`UopStats::shadow_checks`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Replay {
    /// Passes replayed and found equal to the interpreter.
    pub checked: u64,
    /// Passes that could not be replayed (the core has peripheral models) and so were not
    /// compared: a run with `skipped > 0` has not shown exactness.
    pub skipped: u64,
}

/// The counters of one measured run, ready to print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UopReport {
    /// Free text naming the run (core, state); JSON-escaped on output.
    pub label: String,
    pub stats: UopStats,
    /// Which passes `stats` covers.
    pub window: Window,
    /// Passes of the checked engine that were compared; zero for the other engines.
    pub replay: Replay,
    /// Steps per block entry, when the run was profiled (`measure`; the timed benchmark is not).
    pub block_steps: Option<BlockHistogram>,
}

impl UopReport {
    /// A report of `stats` with the given window, no replay and no histogram.
    pub fn new(label: &str, stats: UopStats, window: Window) -> Self {
        UopReport {
            label: label.to_string(),
            stats,
            window,
            replay: Replay::default(),
            block_steps: None,
        }
    }

    /// The report of `core`'s counters since `enable_uop` or the last `uop_reset_stats`: every
    /// pass since then, so the window is [`Window::All`] (a caller that reset the counters at the
    /// first measured pass makes it [`UopReport::new`] with [`Window::Measured`]).
    pub fn from_core(label: &str, core: &Core) -> Self {
        Self::new(label, core.uop_stats(), Window::All)
    }

    /// The measured passes of a run that could not reset the counters between the warm-up and the
    /// measurement (the threaded scheduler): `now` minus the `warm` snapshot taken when the first
    /// measured pass was about to start.
    pub fn measured(label: &str, now: &UopStats, warm: &UopStats) -> Self {
        Self::new(label, now.since(warm), Window::Measured)
    }

    /// With the replay counts of a checked run.
    pub fn with_replay(mut self, replay: Replay) -> Self {
        self.replay = replay;
        self
    }

    /// With the block-length histogram of a profiled run.
    pub fn with_block_steps(mut self, hist: Option<BlockHistogram>) -> Self {
        self.block_steps = hist;
        self
    }

    /// The histogram as one line for a person, when the run was profiled.
    pub fn histogram_text(&self) -> Option<String> {
        self.block_steps.as_ref().map(BlockHistogram::text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(passes: u64, smc: u64) -> UopStats {
        UopStats {
            passes,
            smc,
            maintain_calls: passes,
            steps_uop: 10 * passes,
            ..UopStats::default()
        }
    }

    #[test]
    fn a_report_reads_the_counters_of_a_core_and_says_they_cover_every_pass() {
        let c = Core::new(&[]);
        let r = UopReport::from_core("idle", &c);
        assert_eq!((r.label.as_str(), r.stats), ("idle", UopStats::default()));
        assert_eq!(r.window, Window::All);
        assert_eq!((r.replay, r.block_steps), (Replay::default(), None));
    }

    #[test]
    fn the_measured_report_is_the_counters_after_the_warm_up_snapshot() {
        let warm = stats(3, 7); // the warm-up had a maintenance and 7 smc stores per pass
        let end = UopStats {
            smc: 7, // none after it
            maintain_calls: 3,
            ..stats(8, 7)
        };
        let r = UopReport::measured("steady", &end, &warm);
        assert_eq!(r.window, Window::Measured);
        assert_eq!(
            (r.stats.passes, r.stats.smc, r.stats.maintain_calls),
            (5, 0, 0),
            "the warm-up's smc and maintenance are not the steady state's"
        );
        assert_eq!(r.stats.steps_uop, 50);
        // all of it, when nothing was snapshotted
        assert_eq!(
            UopReport::measured("w", &end, &UopStats::default()).stats,
            end
        );
    }

    #[test]
    fn the_window_names_are_the_json_names() {
        assert_eq!(Window::Measured.name(), "measured");
        assert_eq!(Window::All.name(), "all");
    }

    #[test]
    fn builders_set_only_their_own_field() {
        let base = UopReport::new("b", stats(1, 0), Window::Measured);
        let replay = Replay {
            checked: 4,
            skipped: 1,
        };
        let h = BlockHistogram::from_lengths([3]);
        let r = base
            .clone()
            .with_replay(replay)
            .with_block_steps(Some(h.clone()));
        assert_eq!((r.replay, r.block_steps.as_ref()), (replay, Some(&h)));
        assert_eq!(
            (r.label.as_str(), r.stats, r.window),
            ("b", base.stats, base.window)
        );
        assert_eq!(base.histogram_text(), None);
        assert!(r.histogram_text().unwrap().contains("3:1"));
        assert_eq!(r.with_block_steps(None).histogram_text(), None);
    }

    #[test]
    fn the_model_re_exports_keep_their_old_paths() {
        let org = UopStats {
            passes: 1,
            blocks_run: 520,
            uops: 8478,
            steps_uop: 6280,
            steps_interp_uop: 125,
            ..UopStats::default()
        };
        assert_eq!(
            (
                DISPATCH,
                TRANSITION,
                INTERP_STEP,
                WORK_PER_STEP,
                NATIVE_PER_STEP
            ),
            (6, 38, 100, 42, 38)
        );
        assert_eq!(model_cycles(&org, Tier::One), 341_638);
        assert_eq!(SCHEMA, 1);
    }
}
