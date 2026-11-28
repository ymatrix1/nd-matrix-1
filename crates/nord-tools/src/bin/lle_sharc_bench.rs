//! `lle_sharc_bench` (bean `ns4-lle3`): multi-core SHARC pass benchmark on the lock-free
//! 1-thread-per-core spin-barrier scheduler (`nord_sharc::runner::pass_sched`).
//!
//! ```text
//! cargo run --release -p nord-tools --bin lle_sharc_bench -- \
//!   --core NAME=IMAGE,PRESET[,MSGFILE[,AOT_TAG]]  ...  [--passes 300] [--warmup 16] \
//!   [--engine interp|aot|uop|uop-checked|uop-native] [--uop-families BITS] [--uop-report FILE] \
//!   [--uop-ranges extent|probe]
//! ```
//!
//! `PRESET` is `pno` or `org` (`runner::live_presets`), `MSGFILE` a text of `--msg [PASS:]W1=W2`
//! tokens (as `sharc-sys` takes them), `AOT_TAG` the embedded whole-pass module of the core
//! (`NORD_SHARC_AOT_HOT=TAG=file;...` at build time). With `--engine aot` only cores that have a
//! tag run the AOT blocks; the others stay on the interpreter. Output: one row per core and the
//! scheduler tick, all against the 46.08 us pass period (2 frames at 43 402.34 Hz).
//!
//! The uop engines (D260, `docs/specs/sharc-uop.md`) apply to every core: the booted core is
//! armed with `runner::uop_pass::arm_for` (its code translated, the pass idle pc the stop pc)
//! and each pass runs `lean_pass_uop`. `uop-checked` also replays every pass on the interpreter
//! and panics on a difference: allocating, for exactness runs, not timing. `uop-native` needs
//! `--features nord-sharc/uop-native`; `--uop-families` is its `native::FAM_*` bit set (default 0,
//! call-threaded). `--uop-ranges` picks the code to translate: `extent` (default: the non-zero
//! extent of the VISA window, data tables included, so stores into them count as `smc`) or `probe`
//! (the pcs of `warmup + passes` interpreter passes on a second boot of the core: only code the
//! passes run). After the table, one line of deterministic counters per core is printed and,
//! with `--uop-report FILE`, the same counters are written as a JSON array that
//! `python tools/uop_gates.py FILE` evaluates. The cores run on their own threads, so the
//! scheduler cannot reset their counters between the warm-up and the measured passes: it
//! snapshots them when the first measured pass is about to start (`CoreSeries::uop_at_warmup`)
//! and the report holds the counters at the end minus that snapshot (`"window":"measured"`, the
//! same window as `uop-report`), so the steady-state gates see the measured passes only. The
//! scheduler cannot watch block entries either: the reports carry no `block_steps` histogram
//! (`uop-report` prints it) and no `replay` counts (`uop-checked` panics on a difference).
use nord_sharc::core::aot_prof::aot_prof_enable;
use nord_sharc::runner::parse_number;
use nord_sharc::runner::pass_sched::{run_ticks, summarize, Engine, PassJob, SchedReport};
use nord_sharc::runner::system::{pass_period_s, FRAME_RATE};
use nord_sharc::runner::uop_pass::{engine_named, uop_config, RangeMode};
use nord_sharc::runner::uop_report::{counters_text, UopReport};
use std::fmt::Write as _;

// the AOT profile text lives next to the bench (a module of this binary, not a binary of its own)
#[path = "lle_sharc_bench/profile.rs"]
mod profile;
// one core: its `--core` argument and its armed job
#[path = "lle_sharc_bench/job.rs"]
mod core_job;
use core_job::{job, parse_core, Arming, CoreSpec};

/// The command line.
#[derive(Debug)]
struct Opts {
    specs: Vec<CoreSpec>,
    passes: u32,
    warmup: u32,
    engine: Engine,
    engine_name: String,
    families: u32,
    ranges: RangeMode,
    uop_report: Option<String>,
    profile: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Opts, String> {
    let mut o = Opts {
        specs: Vec::new(),
        passes: 300,
        warmup: 16,
        engine: Engine::Interp,
        engine_name: "interp".into(),
        families: 0,
        ranges: RangeMode::Extent,
        uop_report: None,
        profile: false,
    };
    let mut args = args;
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--core" => o.specs.push(parse_core(&val()?)?),
            "--profile" => o.profile = true,
            "--passes" => o.passes = val()?.parse().map_err(|e| format!("--passes: {e}"))?,
            "--warmup" => o.warmup = val()?.parse().map_err(|e| format!("--warmup: {e}"))?,
            "--engine" => {
                let name = val()?;
                o.engine = engine_named(&name).ok_or(format!("--engine {name:?}"))?;
                o.engine_name = name;
            }
            "--uop-families" => {
                let v = val()?;
                o.families = parse_number(&v).ok_or(format!("bad --uop-families {v}"))?;
            }
            "--uop-ranges" => {
                let v = val()?;
                o.ranges = RangeMode::named(&v)
                    .ok_or(format!("--uop-ranges {v:?}: want extent or probe"))?;
            }
            "--uop-report" => o.uop_report = Some(val()?),
            _ => return Err(format!("unknown argument {a:?}")),
        }
    }
    if o.specs.is_empty() {
        return Err("usage: lle_sharc_bench --core NAME=IMAGE,PRESET[,MSGFILE[,TAG]] ...".into());
    }
    if o.uop_report.is_some() && uop_config(o.engine, 0).is_none() {
        return Err("--uop-report needs --engine uop, uop-checked or uop-native".into());
    }
    if o.ranges != RangeMode::Extent && uop_config(o.engine, 0).is_none() {
        return Err("--uop-ranges probe needs --engine uop, uop-checked or uop-native".into());
    }
    Ok(o)
}

/// One report per core of a finished run: the engine counters at the end minus the snapshot taken
/// when the first measured pass was about to start, so the warm-up is not in them (the window of
/// the report is `measured`). `r.cores` and `r.jobs` are in the same order.
fn measured_reports(r: &SchedReport, engine_name: &str) -> Vec<UopReport> {
    r.cores
        .iter()
        .zip(&r.jobs)
        .map(|(c, j)| {
            let label = format!("{} {engine_name}", j.name);
            UopReport::measured(&label, &j.core.uop_stats(), &c.uop_at_warmup)
        })
        .collect()
}

fn main() -> Result<(), String> {
    let Opts {
        specs,
        passes,
        warmup,
        engine,
        engine_name,
        families,
        ranges,
        uop_report,
        profile,
    } = parse_args(std::env::args().skip(1))?;
    let arming = Arming {
        families,
        ranges,
        probe_passes: warmup + passes,
    };
    let armed = specs
        .iter()
        .map(|s| job(s, engine, arming))
        .collect::<Result<Vec<_>, _>>()?;
    let builds: Vec<_> = armed
        .iter()
        .map(|a| (a.job.name.clone(), a.build))
        .collect();
    let jobs: Vec<PassJob> = armed.into_iter().map(|a| a.job).collect();
    aot_prof_enable(profile);
    let r = run_ticks(jobs, warmup, passes)?;
    let budget_us = pass_period_s(FRAME_RATE) * 1e6;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "engine={engine:?} passes={passes} warmup={warmup} budget={budget_us:.2} us/pass ({} threads)",
        r.cores.len()
    );
    let _ = writeln!(
        out,
        "core | engine | idle | steps/pass | mean us | p50 us | p99 us | worst us | mean/budget | cpu mean us"
    );
    let mut total_mean = 0.0;
    for (c, j) in r.cores.iter().zip(&r.jobs) {
        let (mean, med, p99, worst) = summarize(&c.ns, warmup as usize);
        let steps = summarize(&c.steps, warmup as usize).0;
        let cpu_mean = summarize(&c.cpu_ns, warmup as usize).0;
        total_mean += mean;
        let _ = writeln!(
            out,
            "{} | {:?} | {} | {:.0} | {:.1} | {:.1} | {:.1} | {:.1} | x{:.2} | {:.1}",
            c.name,
            j.engine,
            c.all_idle,
            steps,
            mean / 1e3,
            med / 1e3,
            p99 / 1e3,
            worst / 1e3,
            mean / 1e3 / budget_us,
            cpu_mean / 1e3
        );
    }
    let (mean, med, p99, worst) = summarize(&r.tick_ns, warmup as usize);
    let _ = writeln!(
        out,
        "tick wall (slowest core + barrier): mean {:.1} us, p50 {:.1}, p99 {:.1}, worst {:.1} | total CPU per pass {:.1} us = {:.2} host cores of real time",
        mean / 1e3, med / 1e3, p99 / 1e3, worst / 1e3, total_mean / 1e3, total_mean / 1e3 / budget_us
    );
    if profile {
        let steps = summarize(&r.cores[0].steps, warmup as usize).0;
        out += &profile::profile_text(&r.jobs[0].core.aot_prof_report(), steps);
    }
    if uop_config(engine, 0).is_some() {
        let reports = measured_reports(&r, &engine_name);
        out += &counters_text(&builds, &reports, warmup, passes);
        if let Some(path) = uop_report {
            std::fs::write(&path, UopReport::list_json(&reports))
                .map_err(|e| format!("{path}: {e}"))?;
        }
    }
    print!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use nord_sharc::runner::live_presets::boot_core;
    use nord_sharc::runner::periph_run::PeriphOpts;
    use nord_sharc::runner::uop_pass::arm_for;
    use nord_sharc::runner::uop_report::measure::{synthetic_image, synthetic_preset};
    use nord_sharc::runner::uop_report::Window;

    fn args(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Two armed synthetic cores (the poll program of `uop-report`, no firmware).
    fn armed_jobs(engine: Engine) -> Vec<PassJob> {
        let preset = synthetic_preset();
        ["A", "B"]
            .iter()
            .map(|name| {
                let (image, opts) = (synthetic_image(), PeriphOpts::default());
                let mut core = boot_core(&image, &preset, &opts).unwrap();
                arm_for(&mut core, engine, 0, preset.idle_pc, None)
                    .unwrap()
                    .expect("a uop engine");
                PassJob::from_preset(name, core, &preset, Vec::new(), engine)
            })
            .collect()
    }

    #[test]
    fn the_reports_hold_the_measured_passes_not_the_warm_up() {
        let r = run_ticks(armed_jobs(Engine::Uop), 3, 5).unwrap();
        let reports = measured_reports(&r, "uop");
        assert_eq!(reports.len(), 2);
        for (rep, j) in reports.iter().zip(&r.jobs) {
            assert_eq!(rep.window, Window::Measured);
            assert_eq!(rep.stats.passes, 5, "the 5 measured passes");
            assert_eq!(
                j.core.uop_stats().passes,
                8,
                "the core itself counted the 3 warm-up passes too"
            );
            assert_eq!(rep.label, format!("{} uop", j.name));
            assert!(
                rep.block_steps.is_none(),
                "the scheduler cannot watch entries"
            );
        }
        let json = UopReport::list_json(&reports);
        assert!(json.contains("\"window\":\"measured\""), "{json}");
    }

    #[test]
    fn a_run_without_measured_passes_reports_nothing_of_its_warm_up() {
        // every warm-up pass is excluded: with no measured pass the delta is empty
        let r = run_ticks(armed_jobs(Engine::Uop), 4, 0).unwrap();
        for rep in measured_reports(&r, "uop") {
            assert_eq!(rep.stats.passes, 0);
            assert_eq!(rep.stats.maintain_calls, 0);
            assert_eq!(rep.stats.smc, 0);
        }
    }

    #[test]
    fn engines_and_uop_options_parse_and_the_defaults_are_unchanged() {
        let o = parse_args(args(&["--core", "A=i.bin,org"])).unwrap();
        assert_eq!(
            (o.passes, o.warmup, o.engine, o.families, o.profile),
            (300, 16, Engine::Interp, 0, false)
        );
        assert!(o.uop_report.is_none());
        assert_eq!(o.ranges, RangeMode::Extent);
        for (name, engine) in [
            ("interp", Engine::Interp),
            ("aot", Engine::Aot),
            ("uop", Engine::Uop),
            ("uop-checked", Engine::UopChecked),
            ("uop-native", Engine::UopNative),
        ] {
            let o = parse_args(args(&["--core", "A=i.bin,org", "--engine", name])).unwrap();
            assert_eq!((o.engine, o.engine_name.as_str()), (engine, name));
        }
        let o = parse_args(args(&[
            "--core",
            "A=i.bin,org",
            "--engine",
            "uop-native",
            "--uop-families",
            "0x6",
            "--uop-report",
            "r.json",
            "--passes",
            "50",
            "--uop-ranges",
            "probe",
        ]))
        .unwrap();
        assert_eq!((o.families, o.passes, o.ranges), (6, 50, RangeMode::Probe));
        assert_eq!(o.uop_report.as_deref(), Some("r.json"));
    }

    #[test]
    fn bad_command_lines_are_rejected() {
        for bad in [
            &[][..],                                              // no core
            &["--core", "A=i.bin,org", "--engine", "turbo"],      // unknown engine
            &["--core", "A=i.bin,org", "--engine"],               // no value
            &["--core", "A=i.bin,org", "--uop-families", "x"],    // not a number
            &["--core", "A=i.bin,org", "--uop-report", "r.json"], // counters need a uop engine
            &[
                "--core",
                "A=i.bin,org",
                "--engine",
                "aot",
                "--uop-report",
                "r.json",
            ],
            &["--core", "A=i.bin,org", "--bogus"],
            &[
                "--core",
                "A=i.bin,org",
                "--engine",
                "uop",
                "--uop-ranges",
                "all",
            ], // unknown mode
            &["--core", "A=i.bin,org", "--engine", "uop", "--uop-ranges"], // no value
            &["--core", "A=i.bin,org", "--uop-ranges", "probe"], // a probe needs a uop engine
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }
}
