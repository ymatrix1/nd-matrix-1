//! The human-readable side of a report: one summary line per run ([`UopReport::summary`]), one
//! line about what `enable_uop` built ([`build_summary`]) and the counter block a multi-core run
//! prints after its table ([`counters_text`]). Derived per-pass numbers only; the gate evaluator
//! computes the same ratios from the JSON.
//!
//! OWNER: W1-RUN.
use super::model::{model_cycles, Tier, BUDGET_CYCLES};
use super::{UopReport, Window};
use crate::core::UopBuildReport;

/// `n / d`, 0 for an empty denominator (a run with no passes or no steps has no ratio).
fn ratio(n: u64, d: u64) -> f64 {
    if d == 0 {
        0.0
    } else {
        n as f64 / d as f64
    }
}

impl UopReport {
    /// One line of derived per-pass numbers for a person reading a run (the gate evaluator
    /// computes the same ratios from the JSON): blocks and steps per pass, uops per step, the
    /// shares of `Interp` and `Core::step` steps, `Slow` exits, and the modeled cycles per pass
    /// with their ratio to the real-time budget ([`BUDGET_CYCLES`]).
    pub fn summary(&self) -> String {
        let s = &self.stats;
        let passes = s.passes.max(1);
        let steps = s.steps_uop.saturating_add(s.steps_core_step);
        let per_pass = |tier: Tier| model_cycles(s, tier) / passes;
        let x = |cycles: u64| cycles as f64 / BUDGET_CYCLES as f64;
        format!(
            "{}: passes {}, blocks/pass {:.1}, steps/block {:.1}, uops/step {:.3}, \
interp {:.1}%, core-step {:.1}%, slow/pass {:.1}, guard-fail/pass {:.1}, maintain {}, smc {}, \
poisoned {}, model/pass tier1 {} (x{:.2}) tier2 {} (x{:.2})",
            self.label,
            s.passes,
            ratio(s.blocks_run, passes),
            ratio(s.steps_uop, s.blocks_run),
            ratio(s.uops, s.steps_uop),
            100.0 * ratio(s.steps_interp_uop, steps),
            100.0 * ratio(s.steps_core_step, steps),
            ratio(s.exits[crate::core::Exit::Slow as usize], passes),
            ratio(s.guard_fail.iter().sum(), passes),
            s.maintain_calls,
            s.smc,
            s.poisoned,
            per_pass(Tier::One),
            x(per_pass(Tier::One)),
            per_pass(Tier::Two),
            x(per_pass(Tier::Two)),
        )
    }
}

/// Mean steps of a translated block, 0 when nothing was built: the one static block-length
/// figure the build report carries (`insns` is the steps of every uop of every block, so the
/// ratio does not depend on how many blocks each PEy mode adds). The table of block lengths needs
/// an accessor on the core that does not exist yet (`hist` has the dynamic one).
pub fn mean_block_steps(b: &UopBuildReport) -> f64 {
    if b.blocks == 0 {
        0.0
    } else {
        f64::from(b.insns) / f64::from(b.blocks)
    }
}

/// One line about what `enable_uop` built (the static side of a run; the counters are the
/// dynamic side): blocks and their mean length in steps, uops, instructions and how many of them
/// stay `Interp` uops, and the code pages the write hook watches.
pub fn build_summary(b: &UopBuildReport) -> String {
    format!(
        "built {} blocks (mean {:.1} steps), {} uops, {} insns ({} as Interp uops), {} code pages",
        b.blocks,
        mean_block_steps(b),
        b.uops,
        b.insns,
        b.interp_insns,
        b.pages
    )
}

/// The counter block a multi-core run prints after its table: a header naming which passes the
/// counters cover (the `passes` measured ones, or, for a report of [`Window::All`], also the
/// `warmup` ones), one build line per core that was armed, then one [`UopReport::summary`] per
/// core and, for a profiled run, its block-length histogram.
pub fn counters_text(
    builds: &[(String, Option<UopBuildReport>)],
    reports: &[UopReport],
    warmup: u32,
    passes: u32,
) -> String {
    let measured = reports.iter().all(|r| r.window == Window::Measured);
    let mut out = if measured {
        format!(
            "uop counters (deterministic, the {passes} measured passes; the {warmup} warm-up \
passes are not in them):\n"
        )
    } else {
        format!(
            "uop counters (deterministic, all {} passes, warm-up included):\n",
            u64::from(warmup) + u64::from(passes)
        )
    };
    for (name, build) in builds {
        if let Some(b) = build {
            out += &format!("{name}: {}\n", build_summary(b));
        }
    }
    for r in reports {
        out += &format!("{}\n", r.summary());
        if let Some(h) = r.histogram_text() {
            out += &format!("{}: {h}\n", r.label);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::BlockHistogram;
    use super::*;
    use crate::core::UopStats;

    /// The ORG Core2 reference counters of design 9.1, with distinct exit and guard counts.
    fn golden_report() -> UopReport {
        UopReport::new(
            "ORG Core2",
            UopStats {
                passes: 1,
                blocks_run: 520,
                uops: 8478,
                steps_uop: 6280,
                steps_interp_uop: 125,
                exits: [1, 2, 3, 4, 5, 6],
                guard_fail: [0, 0, 0, 0, 0, 0, 0, 0, 9],
                ..UopStats::default()
            },
            Window::Measured,
        )
    }

    #[test]
    fn the_summary_is_one_line_of_per_pass_numbers() {
        let s = golden_report().summary();
        assert_eq!(
            s,
            "ORG Core2: passes 1, blocks/pass 520.0, steps/block 12.1, uops/step 1.350, \
interp 2.0%, core-step 0.0%, slow/pass 6.0, guard-fail/pass 9.0, maintain 0, smc 0, poisoned 0, \
model/pass tier1 341638 (x1.65) tier2 341638 (x1.65)"
        );
        assert!(!s.contains('\n'));
        // per pass: two passes of the same counters read the same
        let two = UopReport::new(
            "ORG Core2",
            UopStats {
                passes: 2,
                blocks_run: 1040,
                uops: 16956,
                steps_uop: 12560,
                steps_interp_uop: 250,
                exits: [2, 4, 6, 8, 10, 12],
                guard_fail: [0, 0, 0, 0, 0, 0, 0, 0, 18],
                ..UopStats::default()
            },
            Window::Measured,
        );
        assert_eq!(two.summary().replacen("passes 2,", "passes 1,", 1), s);
    }

    #[test]
    fn the_summary_of_an_empty_run_has_no_nan_and_no_division_by_zero() {
        let s = UopReport::new("idle", UopStats::default(), Window::Measured).summary();
        assert!(!s.contains("NaN") && !s.contains("inf"), "{s}");
        assert!(s.starts_with("idle: passes 0, blocks/pass 0.0"), "{s}");
    }

    #[test]
    fn the_build_summary_names_every_field() {
        let b = UopBuildReport {
            blocks: 3,
            uops: 7,
            insns: 9,
            interp_insns: 2,
            pages: 5,
        };
        assert_eq!(
            build_summary(&b),
            "built 3 blocks (mean 3.0 steps), 7 uops, 9 insns (2 as Interp uops), 5 code pages"
        );
    }

    #[test]
    fn the_mean_block_length_is_the_steps_of_the_build_over_its_blocks() {
        let built = |blocks, insns| UopBuildReport {
            blocks,
            insns,
            ..UopBuildReport::default()
        };
        assert_eq!(mean_block_steps(&built(4, 50)), 12.5);
        assert_eq!(mean_block_steps(&built(1, 1)), 1.0);
        assert_eq!(mean_block_steps(&built(0, 0)), 0.0, "nothing built, no NaN");
        assert_eq!(
            mean_block_steps(&built(0, 7)),
            0.0,
            "and no division by zero"
        );
        assert!(build_summary(&built(0, 0)).starts_with("built 0 blocks (mean 0.0 steps),"));
    }

    #[test]
    fn the_counter_text_has_a_build_line_and_a_summary_per_core() {
        let reports = vec![
            UopReport::new("A uop", UopStats::default(), Window::Measured),
            UopReport::new("B uop", UopStats::default(), Window::Measured),
        ];
        let builds = vec![
            ("A".to_string(), Some(UopBuildReport::default())),
            ("B".to_string(), None),
        ];
        let t = counters_text(&builds, &reports, 16, 300);
        assert!(
            t.starts_with(
                "uop counters (deterministic, the 300 measured passes; the 16 warm-up passes \
are not in them):\n"
            ),
            "{t}"
        );
        assert!(t.contains("A: built 0 blocks"), "{t}");
        assert!(
            !t.contains("B: built"),
            "a core that was not armed has no build line"
        );
        assert!(
            t.contains("A uop: passes 0") && t.contains("B uop: passes 0"),
            "{t}"
        );
        assert_eq!(
            t.lines().count(),
            4,
            "header, one build line, two summaries"
        );
    }

    #[test]
    fn a_report_that_covers_the_warm_up_says_so_in_the_header() {
        let reports = vec![
            UopReport::new("A uop", UopStats::default(), Window::Measured),
            UopReport::new("B uop", UopStats::default(), Window::All),
        ];
        let t = counters_text(&[], &reports, 16, 300);
        assert!(
            t.starts_with("uop counters (deterministic, all 316 passes, warm-up included):\n"),
            "one report with the warm-up in it is enough: {t}"
        );
    }

    #[test]
    fn a_profiled_report_prints_its_histogram_under_its_summary() {
        let r = golden_report().with_block_steps(Some(BlockHistogram::from_lengths([12, 12, 1])));
        let t = counters_text(&[], &[r], 2, 4);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 3, "{t}");
        assert!(lines[1].starts_with("ORG Core2: passes 1,"), "{t}");
        assert!(
            lines[2].starts_with("ORG Core2: steps per block entry [1:1 9-12:2] (3 entries"),
            "{t}"
        );
    }
}
