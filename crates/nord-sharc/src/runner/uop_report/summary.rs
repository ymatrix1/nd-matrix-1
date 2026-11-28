//! The human-readable side of a report: one summary line per run ([`UopReport::summary`]), one
//! line about what `enable_uop` built ([`build_summary`]) and the counter block a multi-core run
//! prints after its table ([`counters_text`]). Derived per-pass numbers only; the gate evaluator
//! computes the same ratios from the JSON.
//!
//! OWNER: W1-RUN.
use super::model::{model_cycles, Tier, BUDGET_CYCLES};
use super::UopReport;
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

/// One line about what `enable_uop` built (the static side of a run; the counters are the
/// dynamic side): blocks, uops, instructions and how many of them stay `Interp` uops, and the
/// code pages the write hook watches.
pub fn build_summary(b: &UopBuildReport) -> String {
    format!(
        "built {} blocks, {} uops, {} insns ({} as Interp uops), {} code pages",
        b.blocks, b.uops, b.insns, b.interp_insns, b.pages
    )
}

/// The counter block a multi-core run prints after its table: a header naming how many passes the
/// counters cover (warm-up included when the scheduler had no hook between the phases), one
/// build line per core that was armed, then one [`UopReport::summary`] per core.
pub fn counters_text(
    builds: &[(String, Option<UopBuildReport>)],
    reports: &[UopReport],
    warmup: u32,
    passes: u32,
) -> String {
    let mut out = format!(
        "uop counters (deterministic, all {} passes, warm-up included):\n",
        u64::from(warmup) + u64::from(passes)
    );
    for (name, build) in builds {
        if let Some(b) = build {
            out += &format!("{name}: {}\n", build_summary(b));
        }
    }
    for r in reports {
        out += &format!("{}\n", r.summary());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::UopStats;

    /// The ORG Core2 reference counters of design 9.1, with distinct exit and guard counts.
    fn golden_report() -> UopReport {
        UopReport {
            label: "ORG Core2".into(),
            stats: UopStats {
                passes: 1,
                blocks_run: 520,
                uops: 8478,
                steps_uop: 6280,
                steps_interp_uop: 125,
                exits: [1, 2, 3, 4, 5, 6],
                guard_fail: [0, 0, 0, 0, 0, 0, 0, 0, 9],
                ..UopStats::default()
            },
        }
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
        let two = UopReport {
            label: "ORG Core2".into(),
            stats: UopStats {
                passes: 2,
                blocks_run: 1040,
                uops: 16956,
                steps_uop: 12560,
                steps_interp_uop: 250,
                exits: [2, 4, 6, 8, 10, 12],
                guard_fail: [0, 0, 0, 0, 0, 0, 0, 0, 18],
                ..UopStats::default()
            },
        };
        assert_eq!(two.summary().replacen("passes 2,", "passes 1,", 1), s);
    }

    #[test]
    fn the_summary_of_an_empty_run_has_no_nan_and_no_division_by_zero() {
        let s = UopReport {
            label: "idle".into(),
            stats: UopStats::default(),
        }
        .summary();
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
            "built 3 blocks, 7 uops, 9 insns (2 as Interp uops), 5 code pages"
        );
    }

    #[test]
    fn the_counter_text_has_a_build_line_and_a_summary_per_core() {
        let reports = vec![
            UopReport {
                label: "A uop".into(),
                stats: UopStats::default(),
            },
            UopReport {
                label: "B uop".into(),
                stats: UopStats::default(),
            },
        ];
        let builds = vec![
            ("A".to_string(), Some(UopBuildReport::default())),
            ("B".to_string(), None),
        ];
        let t = counters_text(&builds, &reports, 16, 300);
        assert!(t.starts_with("uop counters (deterministic, all 316 passes, warm-up included):\n"));
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
}
