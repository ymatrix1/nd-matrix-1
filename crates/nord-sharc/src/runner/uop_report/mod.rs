//! Deterministic counter report of the uop engine: the JSON `tools/uop_gates.py` reads, a one-line
//! summary for people, and the cost model that turns counters into modeled host cycles (design
//! sections 9 and 12).
//!
//! OWNER: W1-RUN. Every number is a count, never a time, so `tools/uop_gates.py` can evaluate the
//! structural gates (G2) and the model gates (K3) without a benchmark. The JSON field names are
//! stable (schema [`SCHEMA`], pinned by `the_json_of_schema_1_is_pinned_byte_for_byte` here and
//! by `test_the_rust_golden_report_validates` in `tools/tests/test_uop_gates.py`):
//! [`UopReport::to_json`] is hand-written (this crate has no dependencies) and `nord-tools`'
//! `uop-report` and `lle_sharc_bench` print it. The costs are in [`model`], the single-thread
//! measuring loop that produces steady-state counters in [`measure`].
pub mod measure;
pub mod model;
mod summary;

pub use model::{
    model_cycles, Tier, BUDGET_CYCLES, DISPATCH, INTERP_STEP, NATIVE_PER_STEP, TRANSITION,
    WORK_PER_STEP,
};
pub use summary::{build_summary, counters_text};

use crate::core::{Core, UopStats};

/// Schema version of the JSON; bump when a field is renamed or removed.
pub const SCHEMA: u32 = 1;

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

    /// Several reports as one JSON array (what `tools/uop_gates.py` reads for a multi-core run).
    pub fn list_json(reports: &[UopReport]) -> String {
        let items: Vec<String> = reports.iter().map(UopReport::to_json).collect();
        format!("[{}]", items.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ORG Core2 reference counters of design 9.1.
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

    /// Schema 1 of [`org_core2`] with `exits [1,2,3,4,5,6]` and the label `ORG Core2`. Copied
    /// into `tools/tests/test_uop_gates.py` (`GOLDEN`), which feeds it to the gate evaluator:
    /// change both or neither.
    const GOLDEN: &str = "{\"schema\":1,\"label\":\"ORG Core2\",\"stats\":{\"passes\":1,\
\"blocks_run\":520,\"uops\":8478,\"steps_uop\":6280,\"steps_interp_uop\":125,\
\"steps_core_step\":0,\"steps_native\":0,\"exits\":[1,2,3,4,5,6],\
\"guard_fail\":[0,0,0,0,0,0,0,0,9],\"smc\":0,\"translations\":0,\"poisoned\":0,\
\"maintain_calls\":0,\"shadow_checks\":0,\"shadow_mismatch\":0,\"shadow_skipped\":0},\
\"model\":{\"tier1_cycles\":341638,\"tier2_cycles\":341638}}";

    fn golden_report() -> UopReport {
        UopReport {
            label: "ORG Core2".into(),
            stats: UopStats {
                exits: [1, 2, 3, 4, 5, 6],
                guard_fail: [0, 0, 0, 0, 0, 0, 0, 0, 9],
                ..org_core2()
            },
        }
    }

    #[test]
    fn the_json_of_schema_1_is_pinned_byte_for_byte() {
        assert_eq!(golden_report().to_json(), GOLDEN);
        assert_eq!(SCHEMA, 1);
    }

    #[test]
    fn the_json_has_stable_field_names_and_escapes_the_label() {
        let r = UopReport {
            label: "ORG \"Core2\"\n".into(),
            ..golden_report()
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
    fn every_counter_is_in_the_json_under_its_own_name() {
        // a distinct value per counter: a swapped or dropped field shows
        let s = UopStats {
            passes: 11,
            blocks_run: 12,
            uops: 13,
            steps_uop: 14,
            steps_interp_uop: 15,
            steps_core_step: 16,
            steps_native: 17,
            exits: [21, 22, 23, 24, 25, 26],
            guard_fail: [31, 32, 33, 34, 35, 36, 37, 38, 39],
            smc: 41,
            translations: 42,
            poisoned: 43,
            maintain_calls: 44,
            shadow_checks: 45,
            shadow_mismatch: 46,
            shadow_skipped: 47,
        };
        let j = UopReport {
            label: String::new(),
            stats: s,
        }
        .to_json();
        for (name, v) in [
            ("passes", 11),
            ("blocks_run", 12),
            ("uops", 13),
            ("steps_uop", 14),
            ("steps_interp_uop", 15),
            ("steps_core_step", 16),
            ("steps_native", 17),
            ("smc", 41),
            ("translations", 42),
            ("poisoned", 43),
            ("maintain_calls", 44),
            ("shadow_checks", 45),
            ("shadow_mismatch", 46),
            ("shadow_skipped", 47),
        ] {
            assert!(
                j.contains(&format!("\"{name}\":{v},")) || j.contains(&format!("\"{name}\":{v}}}")),
                "{name} in {j}"
            );
        }
        assert!(j.contains("\"exits\":[21,22,23,24,25,26]"));
        assert!(j.contains("\"guard_fail\":[31,32,33,34,35,36,37,38,39]"));
    }

    #[test]
    fn a_list_of_reports_is_one_json_array() {
        let a = golden_report();
        let b = UopReport {
            label: "PNO Core2".into(),
            ..golden_report()
        };
        assert_eq!(
            UopReport::list_json(&[a.clone(), b.clone()]),
            format!("[{},{}]", a.to_json(), b.to_json())
        );
        assert_eq!(
            UopReport::list_json(std::slice::from_ref(&a)),
            format!("[{}]", a.to_json())
        );
        assert_eq!(UopReport::list_json(&[]), "[]");
    }

    #[test]
    fn a_report_reads_the_counters_of_a_core() {
        let c = Core::new(&[]);
        let r = UopReport::from_core("idle", &c);
        assert_eq!((r.label.as_str(), r.stats), ("idle", UopStats::default()));
    }

    #[test]
    fn the_model_re_exports_keep_their_old_paths() {
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
        assert_eq!(model_cycles(&org_core2(), Tier::One), 341_638);
    }
}
