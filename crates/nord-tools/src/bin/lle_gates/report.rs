//! The report of `lle-gates`: one row per engine or tier from its series, a text table, and one
//! JSON object. Timings are shown only when the host load allowed them (or `--indicative` asks for
//! them marked as such); counters never depend on the host and are always shown.

use crate::gates::{sharc_gates, CoreBands, GateResult};
use crate::sched::{Measured, Series};
use crate::stats::{compare, share_below, Band, RunStats};

/// Tick deadline of the ARM0 loop in microseconds.
pub const TICK_US: f64 = 1000.0;

/// What a group of engines is measured against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A SHARC core pass against the pass period in microseconds.
    Sharc { budget_us: f64, is_org: bool },
    /// The ARM0 tick loop against its 1 ms tick.
    Arm,
}

/// The timing figures of one engine, each over its runs (microseconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub mean: Band,
    pub median: Band,
    pub p99: Band,
    pub max: Band,
    /// Thread CPU time; `None` where the loop records none (ARM).
    pub cpu_median: Option<Band>,
    /// Share of the tick (ARM) or pass budget (SHARC) the median takes.
    pub share: f64,
    /// ARM: share of ticks under 1 ms over all runs.
    pub under_1ms: Option<f64>,
}

/// Counters of a uop engine in the measured passes (G2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Counts {
    pub steps_per_pass: f64,
    pub uop_share: f64,
    pub interp_share: f64,
    pub maintain_calls: u64,
    pub smc: u64,
    pub replay_checked: u64,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub label: String,
    pub runs: usize,
    pub error: Option<String>,
    pub timing: Option<Timing>,
    pub counts: Option<Counts>,
    /// Whether the engine's run is a timing at all (the checked engine's is not).
    pub timed: bool,
    pub steady: bool,
    pub load_max: Option<f64>,
}

pub fn counts_of(rep: &nord_sharc::runner::uop_report::UopReport) -> Counts {
    let s = &rep.stats;
    let total = (s.steps_uop + s.steps_core_step).max(1) as f64;
    let interp = (s.steps_interp_uop + s.steps_core_step) as f64;
    Counts {
        steps_per_pass: total / s.passes.max(1) as f64,
        uop_share: s.steps_uop.saturating_sub(s.steps_interp_uop) as f64 / total,
        interp_share: interp / total,
        maintain_calls: s.maintain_calls,
        smc: s.smc,
        replay_checked: rep.replay.checked,
    }
}

fn bands(series: &Series, f: impl Fn(&RunStats) -> f64, cpu: bool) -> Band {
    let per_run: Vec<f64> = series
        .runs
        .iter()
        .map(|r| f(&RunStats::of(if cpu { &r.cpu_us } else { &r.us })))
        .collect();
    Band::of(&per_run)
}

/// The row of one series. `kind` gives the reference the share is taken against; `with_timing`
/// is false when the load invalidated the timings and they are withheld.
pub fn row_of(series: &Series, kind: Kind, with_timing: bool) -> Row {
    let timed = series.timed && !series.runs.is_empty() && with_timing;
    let reference = match kind {
        Kind::Sharc { budget_us, .. } => budget_us,
        Kind::Arm => TICK_US,
    };
    let timing = timed.then(|| {
        let median = bands(series, |r| r.median, false);
        let all: Vec<f64> = series
            .runs
            .iter()
            .flat_map(|r| r.us.iter().copied())
            .collect();
        Timing {
            mean: bands(series, |r| r.mean, false),
            median,
            p99: bands(series, |r| r.p99, false),
            max: bands(series, |r| r.max, false),
            cpu_median: series
                .runs
                .iter()
                .all(|r| !r.cpu_us.is_empty())
                .then(|| bands(series, |r| r.median, true)),
            share: median.median / reference,
            under_1ms: (kind == Kind::Arm).then(|| share_below(&all, TICK_US)),
        }
    });
    let last = series.runs.last();
    Row {
        label: series.label.clone(),
        runs: series.runs.len(),
        error: series.error.clone(),
        timing,
        counts: last.and_then(|r| r.counters.as_ref()).map(counts_of),
        timed: series.timed,
        steady: !series.runs.is_empty() && series.runs.iter().all(|r| r.steady),
        load_max: series
            .load
            .iter()
            .try_fold(0.0f64, |m, l| l.map(|l| m.max(l))),
    }
}

fn fmt_band(r: &Row, f: impl Fn(&Timing) -> Band) -> String {
    match (&r.timing, r.timed) {
        (Some(t), _) => f(t).text(1),
        (None, false) => "not a timing".into(),
        (None, true) => "withheld".into(),
    }
}

/// The table of one group: a header, one line per engine, and the noise verdicts against the
/// first timed engine.
pub fn table(name: &str, kind: Kind, rows: &[Row], notes: &[String]) -> String {
    let unit = match kind {
        Kind::Sharc { budget_us, .. } => format!("budget {budget_us:.2} us/pass"),
        Kind::Arm => "tick 1000 us".to_string(),
    };
    let mut out = format!("== {name} ({unit}) ==\n");
    out += "engine | runs | mean us | median us | p99 us | max us (each: median [min..max] over runs) | of budget | steps/pass | interp % | maint | smc | steady | load max %\n";
    for r in rows {
        if let Some(e) = &r.error {
            out += &format!("{} | n/a: {e}\n", r.label);
            continue;
        }
        let c = r.counts.as_ref();
        let t = &r.timing;
        let of = t.as_ref().map_or("-".into(), |t| {
            let u = t
                .under_1ms
                .map_or(String::new(), |u| format!(", {:.1}% < 1 ms", u * 100.0));
            format!("{:.1}%{u}", t.share * 100.0)
        });
        out += &format!(
            "{} | {} | {} | {} | {} | {} | {of} | {} | {} | {} | {} | {} | {}\n",
            r.label,
            r.runs,
            fmt_band(r, |t| t.mean),
            fmt_band(r, |t| t.median),
            fmt_band(r, |t| t.p99),
            fmt_band(r, |t| t.max),
            c.map_or("-".into(), |c| format!("{:.0}", c.steps_per_pass)),
            c.map_or("-".into(), |c| format!("{:.2}", c.interp_share * 100.0)),
            c.map_or("-".into(), |c| c.maintain_calls.to_string()),
            c.map_or("-".into(), |c| c.smc.to_string()),
            if r.steady { "yes" } else { "NO" },
            r.load_max.map_or("?".into(), |l| format!("{l:.0}")),
        );
    }
    let timed: Vec<&Row> = rows.iter().filter(|r| r.timing.is_some()).collect();
    if let Some((first, rest)) = timed.split_first() {
        for other in rest {
            let (a, b) = (first.timing.unwrap().median, other.timing.unwrap().median);
            out += &format!("  {}\n", compare(&a, &b).text(&first.label, &other.label));
        }
    }
    for n in notes {
        out += &format!("  note: {n}\n");
    }
    out
}

/// The gates of one SHARC group from its rows (engine labels `interp`, `aot`, `uop`, ...).
pub fn gates_of(name: &str, kind: Kind, rows: &[Row], valid: bool) -> Vec<GateResult> {
    let Kind::Sharc { budget_us, is_org } = kind else {
        return Vec::new();
    };
    let med = |label: &str| {
        rows.iter()
            .find(|r| r.label == label)
            .and_then(|r| r.timing.map(|t| t.median))
    };
    let bands = CoreBands {
        aot: med("aot"),
        uop: med("uop"),
    };
    sharc_gates(name, &bands, budget_us, valid, is_org)
}

/// Whether the group has measured anything at all.
pub fn any_runs(m: &Measured) -> bool {
    m.series.iter().any(|s| !s.runs.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sched::Sample;

    fn series(label: &str, runs: &[&[f64]]) -> Series {
        Series {
            label: label.into(),
            timed: true,
            runs: runs
                .iter()
                .map(|us| Sample {
                    us: us.to_vec(),
                    cpu_us: us.iter().map(|x| x * 0.9).collect(),
                    steady: true,
                    ..Sample::default()
                })
                .collect(),
            load: vec![Some(3.0); runs.len()],
            error: None,
        }
    }

    const SHARC: Kind = Kind::Sharc {
        budget_us: 46.08,
        is_org: true,
    };

    #[test]
    fn a_row_has_bands_over_runs_and_the_share_of_the_budget() {
        let s = series(
            "uop",
            &[
                &[40.0, 42.0, 44.0],
                &[50.0, 52.0, 54.0],
                &[45.0, 46.0, 47.0],
            ],
        );
        let r = row_of(&s, SHARC, true);
        let t = r.timing.unwrap();
        assert_eq!(
            t.median,
            Band {
                min: 42.0,
                median: 46.0,
                max: 52.0
            }
        );
        assert_eq!(t.max.max, 54.0);
        assert!((t.share - 46.0 / 46.08).abs() < 1e-9);
        assert!((t.cpu_median.unwrap().median - 41.4).abs() < 1e-9);
        assert_eq!(r.load_max, Some(3.0));
        assert!(r.steady && t.under_1ms.is_none());
    }

    #[test]
    fn withheld_timings_leave_no_numbers_but_keep_the_counters_and_say_so() {
        let s = series("uop", &[&[40.0], &[41.0]]);
        let r = row_of(&s, SHARC, false);
        assert!(r.timing.is_none());
        let t = table("ORG Core2", SHARC, &[r], &[]);
        assert!(t.contains("withheld"), "{t}");
        assert!(!t.contains("40.0"), "{t}");
    }

    #[test]
    fn arm_rows_report_the_share_of_ticks_under_one_millisecond() {
        let s = series(
            "tier1",
            &[
                &[500.0, 900.0, 1500.0, 700.0],
                &[600.0, 800.0, 1200.0, 400.0],
            ],
        );
        let r = row_of(&s, Kind::Arm, true);
        assert_eq!(r.timing.unwrap().under_1ms, Some(0.75));
        let text = table("ARM0", Kind::Arm, &[r], &[]);
        assert!(text.contains("75.0% < 1 ms"), "{text}");
    }

    #[test]
    fn an_untimed_engine_has_counters_but_no_timing() {
        let mut s = series("uop-checked", &[&[1.0]]);
        s.timed = false;
        assert!(row_of(&s, SHARC, true).timing.is_none());
    }

    #[test]
    fn the_table_says_within_noise_when_the_medians_are_closer_than_the_larger_band() {
        let a = row_of(
            &series("interp", &[&[100.0], &[104.0], &[108.0]]),
            SHARC,
            true,
        );
        let b = row_of(&series("aot", &[&[103.0], &[110.0], &[112.0]]), SHARC, true);
        let c = row_of(&series("uop", &[&[40.0], &[41.0], &[42.0]]), SHARC, true);
        let t = table("ORG Core2", SHARC, &[a, b, c], &[]);
        assert!(t.contains("interp vs aot: within noise"), "{t}");
        assert!(t.contains("uop faster x2."), "{t}");
    }

    #[test]
    fn an_unavailable_engine_prints_its_reason() {
        let s = Series {
            label: "uop-native".into(),
            error: Some("build without feature nord-sharc/uop-native".into()),
            ..Series::default()
        };
        let t = table("ORG Core2", SHARC, &[row_of(&s, SHARC, true)], &[]);
        assert!(t.contains("uop-native | n/a: build without feature"), "{t}");
    }

    #[test]
    fn gates_read_the_aot_and_uop_rows_by_label() {
        let aot = row_of(&series("aot", &[&[160.0], &[161.0], &[162.0]]), SHARC, true);
        let uop = row_of(&series("uop", &[&[50.0], &[51.0], &[52.0]]), SHARC, true);
        let g = gates_of("ORG Core2", SHARC, &[aot, uop], true);
        assert_eq!(g.len(), 3);
        assert!(
            g.iter().all(|x| x.verdict == crate::gates::Verdict::Pass),
            "{g:?}"
        );
        assert!(gates_of("ARM0", Kind::Arm, &[], true).is_empty());
    }
}
