//! Deterministic counter report of the uop engine, and the cost model that turns counters into
//! modeled host cycles (design sections 9 and 12).
//!
//! OWNER: W1-RUN. Every number is a count, never a time, so `tools/uop_gates.py` can evaluate the
//! structural gates (G2) and the model gates (K3) without a benchmark. The JSON field names are
//! stable: the gate evaluator reads them. [`UopReport::to_json`] is hand-written (this crate has
//! no dependencies) and `nord-tools`' `uop-report` prints it.
//!
//! Provenance: the per-class costs below are ASSUMED (design 5: dispatch 6, transition 38,
//! `Interp` 100 host cycles) and are replaced by measurements at gate K0. `WORK_PER_STEP` is the
//! mean of the section 9.1 compute and move rows over the 6,155 non-`Interp` steps of the ORG
//! Core2 reference pass: (2,300 x 89 + 2,900 x 18) / 6,155 = 41.7, ASSUMED.
use crate::core::{Core, UopStats};

/// Schema version of the JSON; bump when a field is renamed or removed.
pub const SCHEMA: u32 = 1;
/// Host cycles per dispatched uop (indirect call, return, `done += steps`). ASSUMED.
pub const DISPATCH: u64 = 6;
/// Host cycles per block transition (exit, link or pc-map probe, entry guard). ASSUMED.
pub const TRANSITION: u64 = 38;
/// Host cycles per `Interp` step (decode lookup plus `exec_cached`), also charged to a step the
/// caller ran through `Core::step`. ASSUMED.
pub const INTERP_STEP: u64 = 100;
/// Host cycles of useful work per non-`Interp` step in Tier 1 (compute and moves mixed). ASSUMED.
pub const WORK_PER_STEP: u64 = 42;
/// Host cycles per step of a native block, dispatch and transitions included (section 9.2: 236k
/// cycles over 6,280 steps = 37.6). ASSUMED.
pub const NATIVE_PER_STEP: u64 = 38;

/// Which tier a model evaluates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Predecoded uops through the handler table.
    One,
    /// Native blocks; steps not run natively are charged as in Tier 1.
    Two,
}

/// Modeled host cycles for the counters `s` (deterministic, ASSUMED costs above).
pub fn model_cycles(s: &UopStats, tier: Tier) -> u64 {
    let interp = s.steps_interp_uop + s.steps_core_step;
    let fast = s.steps_uop.saturating_sub(s.steps_interp_uop);
    let native = match tier {
        Tier::One => 0,
        Tier::Two => s.steps_native.min(fast),
    };
    let tier1 = fast - native;
    // dispatch and block transitions are paid by the Tier 1 share of the fast steps only (the
    // native per-step cost is all-in)
    let share = |x: u64| {
        if fast == 0 {
            x
        } else {
            (u128::from(x) * u128::from(tier1) / u128::from(fast)) as u64
        }
    };
    share(s.uops) * DISPATCH
        + share(s.blocks_run) * TRANSITION
        + interp * INTERP_STEP
        + tier1 * WORK_PER_STEP
        + native * NATIVE_PER_STEP
}

/// The counters of one measured run, ready to print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UopReport {
    /// Free text naming the run (core, state); JSON-escaped on output.
    pub label: String,
    pub stats: UopStats,
}

impl UopReport {
    /// The report of `core`'s counters since `enable_uop` or the last `uop_reset_stats`.
    pub fn from_core(label: &str, core: &Core) -> Self {
        UopReport {
            label: label.to_string(),
            stats: core.uop_stats(),
        }
    }

    /// The report as one JSON object with the stable field names of schema [`SCHEMA`].
    pub fn to_json(&self) -> String {
        let s = &self.stats;
        let list = |v: &[u64]| v.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        let label: String = self
            .label
            .chars()
            .flat_map(|c| match c {
                '"' | '\\' => vec!['\\', c],
                c if c.is_control() => vec![' '],
                c => vec![c],
            })
            .collect();
        format!(
            "{{\"schema\":{SCHEMA},\"label\":\"{label}\",\"stats\":{{\
\"passes\":{},\"blocks_run\":{},\"uops\":{},\"steps_uop\":{},\"steps_interp_uop\":{},\
\"steps_core_step\":{},\"steps_native\":{},\"exits\":[{}],\"guard_fail\":[{}],\"smc\":{},\
\"translations\":{},\"poisoned\":{},\"maintain_calls\":{},\"shadow_checks\":{},\
\"shadow_mismatch\":{},\"shadow_skipped\":{}}},\
\"model\":{{\"tier1_cycles\":{},\"tier2_cycles\":{}}}}}",
            s.passes,
            s.blocks_run,
            s.uops,
            s.steps_uop,
            s.steps_interp_uop,
            s.steps_core_step,
            s.steps_native,
            list(&s.exits),
            list(&s.guard_fail),
            s.smc,
            s.translations,
            s.poisoned,
            s.maintain_calls,
            s.shadow_checks,
            s.shadow_mismatch,
            s.shadow_skipped,
            model_cycles(s, Tier::One),
            model_cycles(s, Tier::Two),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ORG Core2 reference pass of design 9.1: 6,280 steps, 8,478 uops, 520 blocks, 125
    /// `Interp` steps.
    fn org_core2() -> UopStats {
        UopStats {
            passes: 1,
            blocks_run: 520,
            uops: 8478,
            steps_uop: 6280,
            steps_interp_uop: 125,
            ..UopStats::default()
        }
    }

    #[test]
    fn tier_one_model_reproduces_the_section_9_total_of_340k_cycles() {
        let c = model_cycles(&org_core2(), Tier::One);
        assert!(c.abs_diff(340_000) <= 3_000, "{c}");
    }

    #[test]
    fn tier_two_model_reproduces_the_section_9_total_of_236k_cycles_when_every_step_is_native() {
        let s = UopStats {
            steps_native: 6280,
            ..org_core2()
        };
        let c = model_cycles(&s, Tier::Two);
        // 6,155 native steps x 38 + 125 `Interp` steps x 100 = 246k against the table's 236k
        assert!(c.abs_diff(236_000) <= 15_000, "{c}");
        assert!(c < model_cycles(&s, Tier::One), "native is modeled cheaper");
        assert_eq!(
            model_cycles(&s, Tier::One),
            model_cycles(&org_core2(), Tier::One)
        );
    }

    #[test]
    fn steps_run_by_core_step_are_charged_as_interpreter_steps() {
        let s = UopStats {
            steps_core_step: 10,
            ..UopStats::default()
        };
        assert_eq!(model_cycles(&s, Tier::One), 10 * INTERP_STEP);
        assert_eq!(model_cycles(&UopStats::default(), Tier::Two), 0);
    }

    #[test]
    fn the_json_has_stable_field_names_and_escapes_the_label() {
        let r = UopReport {
            label: "ORG \"Core2\"\n".into(),
            stats: UopStats {
                exits: [1, 2, 3, 4, 5, 6],
                guard_fail: [0, 0, 0, 0, 0, 0, 0, 0, 9],
                ..org_core2()
            },
        };
        let j = r.to_json();
        for key in [
            "\"schema\":1",
            "\"label\":\"ORG \\\"Core2\\\" \"",
            "\"passes\":1",
            "\"blocks_run\":520",
            "\"uops\":8478",
            "\"steps_uop\":6280",
            "\"steps_interp_uop\":125",
            "\"steps_core_step\":0",
            "\"steps_native\":0",
            "\"exits\":[1,2,3,4,5,6]",
            "\"guard_fail\":[0,0,0,0,0,0,0,0,9]",
            "\"smc\":0",
            "\"translations\":0",
            "\"poisoned\":0",
            "\"maintain_calls\":0",
            "\"shadow_checks\":0",
            "\"shadow_mismatch\":0",
            "\"shadow_skipped\":0",
            "\"model\":{\"tier1_cycles\":",
            "\"tier2_cycles\":",
        ] {
            assert!(j.contains(key), "missing {key} in {j}");
        }
        assert!(j.starts_with('{') && j.ends_with("}}"));
    }

    #[test]
    fn a_report_reads_the_counters_of_a_core() {
        let c = Core::new(&[]);
        let r = UopReport::from_core("idle", &c);
        assert_eq!((r.label.as_str(), r.stats), ("idle", UopStats::default()));
    }
}
