//! The `--profile` text of `lle_sharc_bench`: shares of the AOT pass cycles by cause, split out of
//! the bench so the main file stays small. Moved verbatim; only the module header and the test
//! are new.
use nord_sharc::core::aot_prof::{ProfReport, ProfRow};
use std::fmt::Write as _;

/// Text of the fallback profile of the first core: shares of the pass cycles by cause and the top
/// instruction classes (`--profile`).
pub fn profile_text(p: &ProfReport, steps_per_pass: f64) -> String {
    let total = p.pass_cycles.max(1) as f64;
    let pct = |c: u64| 100.0 * c as f64 / total;
    let by_cause = |cause: &str| -> (u64, u64) {
        p.rows
            .iter()
            .filter(|r| r.cause == cause)
            .fold((0, 0), |a, r| (a.0 + r.steps, a.1 + r.cycles))
    };
    let mut out = String::new();
    let n = p.passes.max(1) as f64;
    let _ = writeln!(
        out,
        "profile of core 1: {} passes, {:.0} steps/pass, {:.0} cycles/pass",
        p.passes,
        steps_per_pass,
        total / n
    );
    let (gs, gc) = by_cause("generic");
    let (rs, rc) = by_cause("refused");
    let (us, uc) = by_cause("unblocked");
    let (ms, mc) = by_cause("mid-block");
    let native = p.run_cycles.saturating_sub(gc);
    let block_steps = steps_per_pass - (gs + rs + us + ms) as f64 / n;
    let _ = writeln!(
        out,
        "cause | steps/pass | share of steps | share of pass time"
    );
    let _ = writeln!(
        out,
        "native in blocks (batch + memory/move lowering) | {:.0} | {:.1}% | {:.1}%",
        block_steps - gs as f64 / n,
        100.0 * (block_steps - gs as f64 / n) / steps_per_pass,
        pct(native)
    );
    for (name, s, c) in [
        ("generic aot_step inside blocks", gs, gc),
        ("block start refused by entry guard", rs, rc),
        ("pc in no block (interpreter)", us, uc),
        ("pc inside a block after a bail-out (interpreter)", ms, mc),
    ] {
        let _ = writeln!(
            out,
            "{name} | {:.0} | {:.1}% | {:.1}%",
            s as f64 / n,
            100.0 * s as f64 / n / steps_per_pass,
            pct(c)
        );
    }
    for (k, name) in ["interpreter exec_compute_op", "generated Pe kernel"]
        .iter()
        .enumerate()
    {
        let _ = writeln!(
            out,
            "  of native: compute fields via {name} | {:.0} ops/pass | | {:.1}% = {:.0} cycles/op",
            p.compute_ops[k] as f64 / n,
            pct(p.compute_cycles[k]),
            p.compute_cycles[k] as f64 / p.compute_ops[k].max(1) as f64
        );
    }
    let _ = writeln!(
        out,
        "block entry/exit dispatch (try - run) | | | {:.1}%",
        pct(p.try_cycles.saturating_sub(p.run_cycles))
    );
    let other = p.pass_cycles.saturating_sub(p.try_cycles + rc + uc + mc);
    let _ = writeln!(out, "loop glue and idle check | | | {:.1}%", pct(other));
    let _ = writeln!(
        out,
        "top fallback classes by time (class | cause | steps/pass | us-share of pass)"
    );
    let mut rows: Vec<&ProfRow> = p.rows.iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.cycles));
    for r in rows.iter().take(12) {
        let _ = writeln!(
            out,
            "{} | {} | {:.1} | {:.1}%",
            r.class,
            r.cause,
            r.steps as f64 / n,
            pct(r.cycles)
        );
    }
    let _ = writeln!(out, "entry refusals per pass by first failing condition:");
    for (why, c) in &p.refusals {
        let _ = writeln!(out, "{why} | {:.1}", *c as f64 / n);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_profile_prints_every_section_without_dividing_by_zero() {
        let t = profile_text(&ProfReport::default(), 100.0);
        assert!(
            t.starts_with("profile of core 1: 0 passes, 100 steps/pass, 1 cycles/pass"),
            "{t}"
        );
        for section in [
            "cause | steps/pass",
            "native in blocks",
            "block start refused by entry guard",
            "loop glue and idle check",
            "entry refusals per pass by first failing condition:",
        ] {
            assert!(t.contains(section), "missing {section:?} in {t}");
        }
        assert!(!t.contains("NaN") && !t.contains("inf"), "{t}");
    }

    #[test]
    fn a_row_shows_up_with_its_class_cause_and_per_pass_steps() {
        let p = ProfReport {
            passes: 2,
            pass_cycles: 1000,
            rows: vec![ProfRow {
                class: "Flow:JumpDirect".into(),
                cause: "refused",
                steps: 10,
                cycles: 500,
            }],
            ..ProfReport::default()
        };
        let t = profile_text(&p, 50.0);
        assert!(t.contains("Flow:JumpDirect | refused | 5.0 | 50.0%"), "{t}");
    }
}
