//! The JSON of a [`UopReport`]: one object per run, hand-written (this crate has no
//! dependencies), the stable field names of schema [`SCHEMA`].
//!
//! OWNER: W1-RUN. `tools/uop_gates.py` reads it (`validate`); `GOLDEN` below is copied into
//! `tools/tests/test_uop_gates.py`: change both or neither.
use super::model::{model_cycles, Tier};
use super::{UopReport, SCHEMA};

/// `v` as comma-separated decimal numbers.
fn list(v: &[u64]) -> String {
    v.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

/// `text` as the inside of a JSON string: quotes and backslashes escaped, control characters
/// (a newline in a label) turned into spaces.
fn escaped(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            c if c.is_control() => vec![' '],
            c => vec![c],
        })
        .collect()
}

impl UopReport {
    /// The report as one JSON object with the stable field names of schema [`SCHEMA`]. The
    /// `block_steps` member follows `model` and is present only for a profiled run.
    pub fn to_json(&self) -> String {
        let s = &self.stats;
        let hist = self
            .block_steps
            .as_ref()
            .map(|h| format!(",\"block_steps\":{}", h.to_json()))
            .unwrap_or_default();
        format!(
            "{{\"schema\":{SCHEMA},\"label\":\"{}\",\"window\":\"{}\",\
\"replay\":{{\"checked\":{},\"skipped\":{}}},\"stats\":{{\
\"passes\":{},\"blocks_run\":{},\"uops\":{},\"steps_uop\":{},\"steps_interp_uop\":{},\
\"steps_core_step\":{},\"steps_native\":{},\"exits\":[{}],\"guard_fail\":[{}],\"smc\":{},\
\"translations\":{},\"poisoned\":{},\"maintain_calls\":{},\"shadow_checks\":{},\
\"shadow_mismatch\":{},\"shadow_skipped\":{}}},\
\"model\":{{\"tier1_cycles\":{},\"tier2_cycles\":{}}}{hist}}}",
            escaped(&self.label),
            self.window.name(),
            self.replay.checked,
            self.replay.skipped,
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
    use super::super::{BlockHistogram, Replay, Window};
    use super::*;
    use crate::core::UopStats;

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

    /// Schema 1 of [`golden_report`]: `exits [1,2,3,4,5,6]`, the label `ORG Core2`, the measured
    /// window and no replay or histogram. Copied into `tools/tests/test_uop_gates.py` (`GOLDEN`),
    /// which feeds it to the gate evaluator: change both or neither.
    const GOLDEN: &str = "{\"schema\":1,\"label\":\"ORG Core2\",\"window\":\"measured\",\
\"replay\":{\"checked\":0,\"skipped\":0},\"stats\":{\"passes\":1,\
\"blocks_run\":520,\"uops\":8478,\"steps_uop\":6280,\"steps_interp_uop\":125,\
\"steps_core_step\":0,\"steps_native\":0,\"exits\":[1,2,3,4,5,6],\
\"guard_fail\":[0,0,0,0,0,0,0,0,9],\"smc\":0,\"translations\":0,\"poisoned\":0,\
\"maintain_calls\":0,\"shadow_checks\":0,\"shadow_mismatch\":0,\"shadow_skipped\":0},\
\"model\":{\"tier1_cycles\":341638,\"tier2_cycles\":341638}}";

    fn golden_report() -> UopReport {
        UopReport::new(
            "ORG Core2",
            UopStats {
                exits: [1, 2, 3, 4, 5, 6],
                guard_fail: [0, 0, 0, 0, 0, 0, 0, 0, 9],
                ..org_core2()
            },
            Window::Measured,
        )
    }

    #[test]
    fn the_json_of_schema_1_is_pinned_byte_for_byte() {
        assert_eq!(golden_report().to_json(), GOLDEN);
        assert_eq!(SCHEMA, 1);
    }

    /// [`GOLDEN`] with the block-length histogram of the entries 12, 12 and 3 steps: the member
    /// follows `model`. Copied into `tools/tests/test_uop_gates.py` (`GOLDEN_PROFILED`): change
    /// both or neither.
    const GOLDEN_PROFILED: &str = "{\"schema\":1,\"label\":\"ORG Core2\",\"window\":\"measured\",\
\"replay\":{\"checked\":0,\"skipped\":0},\"stats\":{\"passes\":1,\
\"blocks_run\":520,\"uops\":8478,\"steps_uop\":6280,\"steps_interp_uop\":125,\
\"steps_core_step\":0,\"steps_native\":0,\"exits\":[1,2,3,4,5,6],\
\"guard_fail\":[0,0,0,0,0,0,0,0,9],\"smc\":0,\"translations\":0,\"poisoned\":0,\
\"maintain_calls\":0,\"shadow_checks\":0,\"shadow_mismatch\":0,\"shadow_skipped\":0},\
\"model\":{\"tier1_cycles\":341638,\"tier2_cycles\":341638},\
\"block_steps\":{\"edges\":[1,2,3,4,6,8,12,16,24,32,48],\
\"counts\":[0,0,1,0,0,0,2,0,0,0,0,0],\"entries\":3,\"steps\":27,\"chained\":0,\"empty\":0}}";

    #[test]
    fn the_profiled_json_is_pinned_byte_for_byte() {
        let r = golden_report().with_block_steps(Some(BlockHistogram::from_lengths([12, 12, 3])));
        assert_eq!(r.to_json(), GOLDEN_PROFILED);
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
            "\"window\":\"measured\"",
            "\"replay\":{\"checked\":0,\"skipped\":0}",
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
        let j = UopReport::new("", s, Window::All).to_json();
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
    fn the_window_the_replay_and_the_histogram_are_report_fields_of_their_own() {
        // the engine's `shadow_checks` (blocks) and the pass counts of the checked run are two
        // fields, and the window names the passes the counters cover
        let r = golden_report()
            .with_replay(Replay {
                checked: 7,
                skipped: 2,
            })
            .with_block_steps(Some(BlockHistogram::from_lengths([12])));
        let j = UopReport {
            window: Window::All,
            stats: UopStats {
                shadow_checks: 99,
                ..r.stats
            },
            ..r
        }
        .to_json();
        assert!(j.contains("\"window\":\"all\""), "{j}");
        assert!(
            j.contains("\"replay\":{\"checked\":7,\"skipped\":2}"),
            "{j}"
        );
        assert!(
            j.contains("\"shadow_checks\":99,"),
            "the engine meaning is kept: {j}"
        );
        assert!(
            j.ends_with(
                ",\"block_steps\":{\"edges\":[1,2,3,4,6,8,12,16,24,32,48],\
\"counts\":[0,0,0,0,0,0,1,0,0,0,0,0],\"entries\":1,\"steps\":12,\"chained\":0,\"empty\":0}}"
            ),
            "{j}"
        );
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
    fn a_report_prints_as_one_line() {
        let j = golden_report()
            .with_block_steps(Some(BlockHistogram::new()))
            .to_json();
        assert!(!j.contains('\n') && j.starts_with('{') && j.ends_with('}'));
    }
}
