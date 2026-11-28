//! JSON and load-line output of `lle-gates` (rows and load log of `report.rs`).

use crate::load::LoadLog;
use crate::report::Row;
use crate::stats::Band;
use serde_json::{json, Value};

fn band_json(b: &Band) -> Value {
    json!({"min": b.min, "median": b.median, "max": b.max})
}

pub fn row_json(r: &Row) -> Value {
    let timing = r.timing.as_ref().map(|t| {
        json!({
            "mean_us": band_json(&t.mean), "median_us": band_json(&t.median),
            "p99_us": band_json(&t.p99), "max_us": band_json(&t.max),
            "cpu_median_us": t.cpu_median.as_ref().map(band_json), "share_of_budget": t.share,
            "share_ticks_under_1ms": t.under_1ms,
        })
    });
    let counts = r.counts.as_ref().map(|c| {
        json!({
            "steps_per_pass": c.steps_per_pass, "uop_step_share": c.uop_share,
            "interp_step_share": c.interp_share, "maintain_calls": c.maintain_calls,
            "smc": c.smc, "replay_checked": c.replay_checked,
        })
    });
    json!({
        "engine": r.label, "runs": r.runs, "error": r.error, "steady": r.steady,
        "load_max_pct": r.load_max, "timing": timing, "counters": counts,
    })
}

pub fn load_json(l: &LoadLog, limit: f64) -> Value {
    json!({
        "limit_pct": limit, "before": l.before, "after": l.after, "runs": l.runs,
        "peak": l.peak(), "flagged_runs": l.flagged_runs(limit), "valid": l.valid(limit),
    })
}

/// The load line of the table: before, after, peak, and the flagged runs.
pub fn load_text(l: &LoadLog, limit: f64) -> String {
    let f = |x: Option<f64>| x.map_or("unknown".to_string(), |v| format!("{v:.0}%"));
    let flagged = l.flagged_runs(limit);
    let verdict = if l.valid(limit) {
        "timings valid".to_string()
    } else {
        format!(
            "TIMINGS NOT VALID: {} run(s) flagged above {limit:.0}% or unknown, peak {}",
            flagged.len(),
            f(l.peak())
        )
    };
    format!(
        "host background load: before {}, after {}, peak {} (limit {limit:.0}%): {verdict}\n",
        f(l.before),
        f(l.after),
        f(l.peak())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{row_of, Kind};
    use crate::sched::{Sample, Series};

    const SHARC: Kind = Kind::Sharc {
        budget_us: 46.08,
        is_org: true,
    };

    #[test]
    fn the_load_line_names_flagged_runs_and_the_json_carries_them() {
        let l = LoadLog {
            before: Some(2.0),
            runs: vec![Some(3.0), Some(60.0)],
            after: Some(4.0),
        };
        let t = load_text(&l, 25.0);
        assert!(
            t.contains("TIMINGS NOT VALID: 1 run(s)") && t.contains("peak 60%"),
            "{t}"
        );
        let j = load_json(&l, 25.0);
        assert_eq!(j["flagged_runs"], json!([1]));
        assert_eq!(j["valid"], false);
        let ok = LoadLog {
            runs: vec![Some(3.0)],
            ..l
        };
        assert!(load_text(&ok, 25.0).contains("timings valid"));
    }

    #[test]
    fn rows_serialise_with_null_for_what_was_not_measured() {
        let r = row_of(
            &Series {
                label: "uop".into(),
                timed: true,
                runs: vec![Sample {
                    us: vec![40.0],
                    cpu_us: vec![36.0],
                    steady: true,
                    ..Sample::default()
                }],
                load: vec![Some(3.0)],
                error: None,
            },
            SHARC,
            false,
        );
        let j = row_json(&r);
        assert_eq!(j["engine"], "uop");
        assert!(j["timing"].is_null() && j["counters"].is_null());
    }
}
