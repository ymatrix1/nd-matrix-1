//! The run behind `lle-gates`: which groups exist for the given files, the measurement of each,
//! and the text, JSON and counter file that come out.

use crate::args::{Args, Only};
use crate::arm::{self, TIERS};
use crate::emit;
use crate::report::{self, any_runs, Kind, Row};
use crate::sched::{alternate, Contender, Measured};
use crate::sharc::{self, Source, ENGINES};
use nord_arm::machine::Machine;
use nord_sharc::runner::system::{pass_period_s, FRAME_RATE};
use nord_sharc::runner::uop_pass::RangeMode;
use nord_sharc::runner::uop_report::UopReport;
use nord_tools::arm_bench::boot;
use serde_json::{json, Value};
use std::path::Path;
use std::rc::Rc;

/// What the tool prints and how it exits.
pub struct Outcome {
    pub text: String,
    pub json: Value,
    /// `UopReport::list_json` of every counter set, for `python tools/uop_gates.py`.
    pub uop_reports: String,
    pub code: i32,
}

fn budget_us() -> f64 {
    pass_period_s(FRAME_RATE) * 1e6
}

/// A group of contenders measured together.
struct Group {
    name: String,
    kind: Kind,
    contenders: Vec<Contender>,
}

/// A part that cannot run here and why.
struct Skip(String);

fn missing(paths: &[&Path]) -> Option<Skip> {
    let gone: Vec<String> = paths
        .iter()
        .filter(|p| !p.is_file())
        .map(|p| p.display().to_string())
        .collect();
    (!gone.is_empty()).then(|| Skip(format!("missing {}", gone.join(", "))))
}

fn sharc_group(a: &Args, name: &str, source: Source, is_org: bool) -> Group {
    let ranges = RangeMode::named(&a.ranges).unwrap_or(RangeMode::Extent);
    let contenders = a
        .engines
        .iter()
        .filter_map(|n| ENGINES.iter().find(|e| e.name == n))
        .map(|e| sharc::contender(source.clone(), *e, ranges, a.passes, a.warmup))
        .collect();
    Group {
        name: name.into(),
        kind: Kind::Sharc {
            budget_us: budget_us(),
            is_org,
        },
        contenders,
    }
}

fn spec(path: &Path, preset: &str, tag: &Option<String>) -> Source {
    Source::Image {
        image: path.display().to_string(),
        preset: preset.into(),
        tag: tag.clone(),
    }
}

/// The groups that can run with the given files, and the parts that cannot.
fn groups(a: &Args) -> (Vec<Group>, Vec<(String, Skip)>) {
    let (mut gs, mut skips) = (Vec::new(), Vec::new());
    if a.only != Only::Arm {
        if a.synthetic {
            gs.push(sharc_group(
                a,
                "synthetic poll core",
                Source::Synthetic,
                false,
            ));
        } else {
            for (name, path, preset, tag, org) in [
                ("ORG Core2", a.org_path(), "org", &a.aot_tag_org, true),
                ("PNO Core2", a.pno_path(), "pno", &a.aot_tag_pno, false),
            ] {
                match missing(&[&path]) {
                    Some(s) => skips.push((name.to_string(), s)),
                    None => gs.push(sharc_group(a, name, spec(&path, preset, tag), org)),
                }
            }
        }
    }
    if a.only != Only::Sharc && !a.synthetic {
        let [code, sram, ee] = a.arm_paths();
        match missing(&[&code, &sram, &ee]) {
            Some(s) => skips.push(("ARM0 tick loop".into(), s)),
            None => gs.push(arm_group(a, &code, &sram, &ee)),
        }
    }
    (gs, skips)
}

fn arm_group(a: &Args, code: &Path, sram: &Path, ee: &Path) -> Group {
    let rd = |p: &Path| std::fs::read(p).unwrap_or_default();
    let base: Rc<Machine> = Rc::new(boot(&rd(code), &rd(sram), &rd(ee)));
    let contenders = a
        .tiers
        .iter()
        .filter_map(|n| TIERS.iter().find(|t| t.0 == n))
        .map(|(n, t)| arm::contender(base.clone(), n, *t, a.tick_insns, a.phase_scale))
        .collect();
    Group {
        name: "ARM0 tick loop".into(),
        kind: Kind::Arm,
        contenders,
    }
}

/// Sets of the measured runs, in the order of the groups.
fn counter_reports(group: &str, m: &Measured) -> Vec<UopReport> {
    m.series
        .iter()
        .filter_map(|s| {
            let mut r = s.runs.last()?.counters.clone()?;
            r.label = format!("{group} {}", s.label);
            Some(r)
        })
        .collect()
}

/// Everything: skip check, early load refusal, the groups, the report.
pub fn run(a: &Args, sampler: &dyn Fn() -> Option<f64>) -> Outcome {
    let (mut gs, skips) = groups(a);
    let mut text = String::new();
    let mut json = json!({"schema": 1, "tool": "lle-gates", "status": "ok"});
    for (name, Skip(why)) in &skips {
        text += &format!("SKIPPED {name}: {why} (looked under {})\n", a.fw.display());
    }
    json["skipped"] = json!(skips
        .iter()
        .map(|(n, s)| json!({"part": n, "why": s.0}))
        .collect::<Vec<_>>());
    if gs.is_empty() {
        json["status"] = json!("skipped");
        text +=
            "nothing to measure: no firmware under --fw (use --synthetic for the wiring check)\n";
        return Outcome {
            text,
            json,
            uop_reports: "[]".into(),
            code: 0,
        };
    }
    // refuse early: no point running minutes of passes on a loaded host
    let first = sampler();
    if !a.indicative && !crate::load::load_ok(first, a.load_limit) {
        text += &format!(
            "REFUSED: host background load {} is above the {:.0}% limit (or unknown); no timing was taken. \
             Close other work and run again, or pass --indicative for numbers marked as such.\n",
            first.map_or("unknown".into(), |l| format!("{l:.0}%")),
            a.load_limit
        );
        json["status"] = json!("refused");
        json["load"] = json!({"before": first, "limit_pct": a.load_limit});
        return Outcome {
            text,
            json,
            uop_reports: "[]".into(),
            code: 4,
        };
    }
    let (mut groups_json, mut reports, mut code) = (Vec::new(), Vec::new(), 0);
    let mut all_valid = true;
    for g in gs.iter_mut() {
        let m = alternate(&mut g.contenders, a.runs, sampler);
        let valid = m.load.valid(a.load_limit);
        all_valid &= valid;
        let show = valid || a.indicative;
        let rows: Vec<Row> = m
            .series
            .iter()
            .map(|s| report::row_of(s, g.kind, show))
            .collect();
        let mut notes = Vec::new();
        if !valid && a.indicative {
            notes.push("INDICATIVE: host load above the limit, no gate verdict".to_string());
        }
        for s in m.series.iter().filter(|s| !s.timed && !s.runs.is_empty()) {
            notes.push(format!(
                "{} is an exactness run: counters only, no timing",
                s.label
            ));
        }
        if !rows.iter().any(|r| r.steady) && any_runs(&m) {
            code = 3;
        }
        // the synthetic core is a wiring check: its 11-step pass says nothing about a gate
        let gate_results = if a.synthetic {
            notes.push("synthetic core: wiring check, no gate verdicts".to_string());
            Vec::new()
        } else {
            report::gates_of(&g.name, g.kind, &rows, valid)
        };
        text += &report::table(&g.name, g.kind, &rows, &notes);
        text += &emit::load_text(&m.load, a.load_limit);
        for r in &gate_results {
            text += &format!("  {}\n", r.line());
        }
        text.push('\n');
        reports.extend(counter_reports(&g.name, &m));
        groups_json.push(json!({
            "name": g.name, "rows": rows.iter().map(emit::row_json).collect::<Vec<_>>(),
            "load": emit::load_json(&m.load, a.load_limit),
            "gates": gate_results.iter().map(|r| json!({"gate": r.gate, "value": r.value, "limit": r.limit, "verdict": r.line()})).collect::<Vec<_>>(),
        }));
    }
    if !all_valid && !a.indicative {
        json["status"] = json!("refused");
        text += "REFUSED: host load rose above the limit during the measurement; timings are withheld.\n";
        code = 4;
    }
    json["params"] = json!({"runs": a.runs, "passes": a.passes, "warmup": a.warmup, "budget_us": budget_us(), "bank": a.bank});
    json["groups"] = json!(groups_json);
    Outcome {
        text,
        json,
        uop_reports: UopReport::list_json(&reports),
        code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{parse_args, tests::args};

    fn synthetic(extra: &[&str]) -> Args {
        let mut v = vec![
            "--synthetic",
            "--runs",
            "2",
            "--passes",
            "20",
            "--warmup",
            "2",
        ];
        v.extend_from_slice(extra);
        parse_args(args(&v)).unwrap()
    }

    #[test]
    fn a_synthetic_smoke_run_prints_the_table_json_and_counters_without_firmware() {
        let o = run(
            &synthetic(&["--engines", "interp,uop,uop-checked"]),
            &|| Some(2.0),
        );
        assert_eq!(o.code, 0, "{}", o.text);
        assert!(
            o.text
                .contains("== synthetic poll core (budget 46.08 us/pass) =="),
            "{}",
            o.text
        );
        assert!(o.text.contains("timings valid"), "{}", o.text);
        assert!(
            o.text.contains("uop-checked is an exactness run"),
            "{}",
            o.text
        );
        let rows = o.json["groups"][0]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows[1]["timing"]["median_us"]["median"].as_f64().unwrap() > 0.0);
        assert_eq!(rows[1]["counters"]["maintain_calls"], 0);
        assert_eq!(rows[1]["counters"]["smc"], 0);
        assert!(
            rows[2]["timing"].is_null()
                && rows[2]["counters"]["replay_checked"].as_u64().unwrap() > 0
        );
        // the counter file is exactly what tools/uop_gates.py reads
        let v: Value = serde_json::from_str(&o.uop_reports).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2, "uop and uop-checked");
        assert_eq!(v[0]["window"], "measured");
        assert!(v[0]["label"]
            .as_str()
            .unwrap()
            .starts_with("synthetic poll core"));
    }

    #[test]
    fn a_loaded_host_refuses_timings_before_measuring_anything() {
        let o = run(&synthetic(&[]), &|| Some(80.0));
        assert_eq!(o.code, 4);
        assert!(
            o.text.contains("REFUSED") && o.text.contains("80%"),
            "{}",
            o.text
        );
        assert_eq!(o.json["status"], "refused");
        assert!(o.json.get("groups").is_none(), "no group was measured");
        let unknown = run(&synthetic(&[]), &|| None);
        assert_eq!(unknown.code, 4, "an unknown load is not below the limit");
    }

    #[test]
    fn load_that_rises_during_the_run_withholds_the_timings_after_the_fact() {
        use std::cell::Cell;
        let n = Cell::new(0u32);
        // calm at the start, busy from the fourth sample on
        let sampler = || {
            n.set(n.get() + 1);
            Some(if n.get() < 4 { 2.0 } else { 70.0 })
        };
        let o = run(&synthetic(&["--engines", "interp,uop"]), &sampler);
        assert_eq!(o.code, 4, "{}", o.text);
        assert!(
            o.text.contains("withheld") && o.text.contains("TIMINGS NOT VALID"),
            "{}",
            o.text
        );
        assert!(o.json["groups"][0]["rows"][0]["timing"].is_null());
        assert!(
            o.json["groups"][0]["rows"][1]["counters"]["maintain_calls"].is_u64(),
            "counters stay"
        );
    }

    #[test]
    fn indicative_prints_loaded_timings_marked_and_without_gate_verdicts() {
        let o = run(&synthetic(&["--engines", "uop", "--indicative"]), &|| {
            Some(90.0)
        });
        assert_eq!(o.code, 0, "{}", o.text);
        assert!(o.text.contains("INDICATIVE"), "{}", o.text);
        assert!(o.json["groups"][0]["rows"][0]["timing"]["median_us"].is_object());
        assert!(
            o.text.contains("no gate verdict"),
            "a loaded run decides nothing: {}",
            o.text
        );
        assert!(
            !o.text.contains("PASS") && !o.text.contains("FAIL"),
            "{}",
            o.text
        );
    }

    #[test]
    fn missing_firmware_is_a_clean_skip_that_names_the_files() {
        let a = parse_args(args(&["--fw", "/nonexistent/lle-gates-fw"])).unwrap();
        let o = run(&a, &|| Some(1.0));
        assert_eq!(o.code, 0);
        assert_eq!(o.json["status"], "skipped");
        assert!(
            o.text.contains(
                "SKIPPED ORG Core2: missing /nonexistent/lle-gates-fw/ORG/core2_sram.bin"
            ),
            "{}",
            o.text
        );
        assert!(
            o.text.contains("SKIPPED ARM0 tick loop") && o.text.contains("bank0_CODE.bin"),
            "{}",
            o.text
        );
        assert_eq!(o.json["skipped"].as_array().unwrap().len(), 3);
    }
}
