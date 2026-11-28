//! Wall-clock gates of `lle-gates`, from the rows of one SHARC core. The structural gates (G2 counts,
//! K3 model) stay in `tools/uop_gates.py`, which reads the counters this tool writes with
//! `--uop-reports` (counts only, so the verdict does not depend on the host).
//!
//! The rows are `docs/research/sharc-compiler-design.md` section 12 (G3 and G3-RT); every threshold
//! below is that table's number, an ASSUMED decision threshold and not a measurement.

use crate::stats::{compare, Band, Compare};

/// G3: the uop pass must be at least this much faster than the v1 AOT pass (design row G3).
pub const G3_MIN_SPEEDUP: f64 = 1.4;
/// G3: ORG Core2 pass in microseconds at that speedup (v1 is 161 us; 161 / 1.4 = 115).
pub const G3_ORG_MAX_US: f64 = 115.0;
/// G3-RT: a steady pass may take this many budgets (design row G3-RT).
pub const G3_RT_BUDGETS: f64 = 1.3;

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Pass,
    Fail,
    /// The number is there but the host was loaded: no verdict.
    Indicative,
    /// A row the gate needs is missing.
    NotApplicable(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct GateResult {
    pub gate: &'static str,
    pub value: String,
    pub limit: String,
    pub verdict: Verdict,
}

impl GateResult {
    pub fn line(&self) -> String {
        let v = match &self.verdict {
            Verdict::Pass => "PASS".to_string(),
            Verdict::Fail => "FAIL".to_string(),
            Verdict::Indicative => "INDICATIVE (host loaded)".to_string(),
            Verdict::NotApplicable(why) => format!("n/a: {why}"),
        };
        format!("{v} {} = {} (limit {})", self.gate, self.value, self.limit)
    }
}

/// The bands (median of per-run median pass time, microseconds) of the engines of one core.
#[derive(Clone, Copy, Debug, Default)]
pub struct CoreBands {
    pub aot: Option<Band>,
    pub uop: Option<Band>,
}

fn decide(ok: bool, valid: bool) -> Verdict {
    match (valid, ok) {
        (false, _) => Verdict::Indicative,
        (true, true) => Verdict::Pass,
        (true, false) => Verdict::Fail,
    }
}

/// G3, G3 absolute (ORG only) and G3-RT for one core. `valid` is false when the host load made
/// the timings unusable: the values are then reported as indicative only.
pub fn sharc_gates(
    core: &str,
    b: &CoreBands,
    budget_us: f64,
    valid: bool,
    is_org: bool,
) -> Vec<GateResult> {
    let mut out = Vec::new();
    let Some(uop) = b.uop else {
        let why = || Verdict::NotApplicable("no uop row".into());
        for gate in ["G3 speedup over AOT", "G3-RT pass vs budget"] {
            out.push(GateResult {
                gate,
                value: core.into(),
                limit: String::new(),
                verdict: why(),
            });
        }
        return out;
    };
    out.push(match b.aot {
        None => GateResult {
            gate: "G3 speedup over AOT",
            value: core.into(),
            limit: format!(">= x{G3_MIN_SPEEDUP}"),
            verdict: Verdict::NotApplicable("no aot row (needs an AOT tag)".into()),
        },
        Some(aot) => {
            let ratio = aot.median / uop.median.max(f64::MIN_POSITIVE);
            let clear = matches!(compare(&uop, &aot), Compare::AFaster(_));
            GateResult {
                gate: "G3 speedup over AOT",
                value: format!(
                    "x{ratio:.2} ({core}, ranges {})",
                    if clear { "apart" } else { "overlap" }
                ),
                limit: format!(">= x{G3_MIN_SPEEDUP}, ranges apart"),
                verdict: decide(ratio >= G3_MIN_SPEEDUP && clear, valid),
            }
        }
    });
    if is_org {
        out.push(GateResult {
            gate: "G3 ORG pass",
            value: format!("{:.1} us", uop.median),
            limit: format!("<= {G3_ORG_MAX_US} us"),
            verdict: decide(uop.median <= G3_ORG_MAX_US, valid),
        });
    }
    let rt = G3_RT_BUDGETS * budget_us;
    out.push(GateResult {
        gate: "G3-RT pass vs budget",
        value: format!("{:.1} us ({core})", uop.median),
        limit: format!("<= {rt:.1} us ({G3_RT_BUDGETS} x budget)"),
        verdict: decide(uop.median <= rt, valid),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(lo: f64, med: f64, hi: f64) -> Band {
        Band {
            min: lo,
            median: med,
            max: hi,
        }
    }

    fn find<'a>(r: &'a [GateResult], gate: &str) -> &'a GateResult {
        r.iter().find(|g| g.gate == gate).unwrap()
    }

    #[test]
    fn a_clear_speedup_and_a_fast_pass_pass_every_gate() {
        let b = CoreBands {
            aot: Some(band(158.0, 161.0, 165.0)),
            uop: Some(band(54.0, 55.0, 57.0)),
        };
        let r = sharc_gates("ORG Core2", &b, 46.08, true, true);
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|g| g.verdict == Verdict::Pass), "{r:?}");
        assert!(find(&r, "G3 speedup over AOT").value.starts_with("x2.93"));
    }

    #[test]
    fn a_speedup_inside_the_noise_does_not_pass_even_when_the_ratio_is_high_enough() {
        let b = CoreBands {
            aot: Some(band(60.0, 100.0, 150.0)),
            uop: Some(band(50.0, 60.0, 110.0)),
        };
        let r = sharc_gates("PNO Core2", &b, 46.08, true, false);
        let g = find(&r, "G3 speedup over AOT");
        assert!(g.value.contains("overlap"), "{}", g.value);
        assert_eq!(g.verdict, Verdict::Fail);
    }

    #[test]
    fn a_speedup_below_1_4_and_a_slow_pass_fail() {
        let b = CoreBands {
            aot: Some(band(159.0, 161.0, 162.0)),
            uop: Some(band(129.0, 130.0, 131.0)),
        };
        let r = sharc_gates("ORG Core2", &b, 46.08, true, true);
        assert_eq!(find(&r, "G3 speedup over AOT").verdict, Verdict::Fail);
        assert_eq!(find(&r, "G3 ORG pass").verdict, Verdict::Fail);
        assert_eq!(find(&r, "G3-RT pass vs budget").verdict, Verdict::Fail);
    }

    #[test]
    fn the_rt_limit_is_1_3_budgets_and_the_org_row_exists_only_for_org() {
        let b = CoreBands {
            aot: None,
            uop: Some(band(59.0, 59.9, 60.0)),
        };
        let r = sharc_gates("PNO Core2", &b, 46.08, true, false);
        assert_eq!(r.len(), 2, "no ORG row for another core");
        assert_eq!(
            find(&r, "G3-RT pass vs budget").verdict,
            Verdict::Pass,
            "59.9 <= 59.904"
        );
        assert!(matches!(
            find(&r, "G3 speedup over AOT").verdict,
            Verdict::NotApplicable(_)
        ));
    }

    #[test]
    fn a_loaded_host_gives_indicative_numbers_and_no_verdict() {
        let b = CoreBands {
            aot: Some(band(160.0, 161.0, 162.0)),
            uop: Some(band(50.0, 51.0, 52.0)),
        };
        for g in sharc_gates("ORG Core2", &b, 46.08, false, true) {
            assert_eq!(g.verdict, Verdict::Indicative, "{g:?}");
            assert!(g.line().contains("INDICATIVE"));
        }
    }

    #[test]
    fn without_a_uop_row_nothing_is_decided() {
        let r = sharc_gates("ORG Core2", &CoreBands::default(), 46.08, true, true);
        assert!(r
            .iter()
            .all(|g| matches!(g.verdict, Verdict::NotApplicable(_))));
        assert!(r[0].line().starts_with("n/a"));
    }
}
