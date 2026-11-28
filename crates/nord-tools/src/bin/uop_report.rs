//! Deterministic counter report of the SHARC uop engine (D260, `docs/specs/sharc-uop.md`).
//!
//! ```text
//! cargo run --release -p nord-tools --bin uop-report -- [--passes N] [--warmup N] [--label TEXT]
//!     [--engine uop|uop-checked|uop-native] [--uop-families BITS] [--ranges extent|probe]
//!     [--image FILE --preset NAME [--msgs FILE]]
//! ```
//!
//! Boots a core, arms the engine (`runner::uop_pass::arm_for`: the code of the booted core is
//! translated, the pass idle pc is the stop pc), runs `--warmup` plus `--passes` passes on this
//! thread and prints one JSON object with the counters of the measured passes and the modeled
//! cycles. The field names are stable (`runner/uop_report`, schema 1): `tools/uop_gates.py` reads
//! this output and evaluates the structural gates. The report carries counts only, never a time,
//! so two runs print identical text.
//!
//! Without `--image` the core is the synthetic poll program of `runner::uop_report::measure` (ten
//! NOPs and a jump, 11 steps per pass, no firmware): the schema and wiring check. With
//! `--image FILE --preset NAME` (`pno`, `org`, ... as in `lle_sharc_bench`; the image is the
//! user's own raw core SRAM, never committed) the same loop reports the real pass of that core,
//! e.g. ORG Core2 for gate G2. `--msgs FILE` delivers ARM messages as `lle_sharc_bench` does. The
//! default engine is `uop`; `uop-native` needs `--features nord-sharc/uop-native`. `--ranges`
//! picks the code to translate: `extent` (default, the non-zero extent of the VISA window: it also
//! takes data tables for code, so stores into them count as `smc`) or `probe` (the pcs of
//! `warmup + passes` interpreter passes on a second boot of the same core: no data page, so G2
//! `smc` can be 0; code the probe never ran stays in the interpreter).
//!
//! `--engine uop-checked` replays every pass on the interpreter and fails (exit 2) on a difference.
//! A core with peripheral models cannot be replayed; such passes are not compared and are counted
//! apart, because a run that compared nothing has shown nothing.
//!
//! Exit code 0 on success, 2 on a usage error, a boot or arming failure or a difference found by
//! `uop-checked`, 3 when a pass did not return to its poll loop or `uop-checked` could not compare
//! a pass (the report is still printed: its counters are not a steady state, or not a check).

use nord_sharc::runner::live_presets::{boot_core, preset_named, Preset};
use nord_sharc::runner::msg::{parse_msg_text, Msg};
use nord_sharc::runner::parse_number;
use nord_sharc::runner::pass_sched::{Engine, PassJob};
use nord_sharc::runner::periph_run::PeriphOpts;
use nord_sharc::runner::uop_pass::{arm_for, engine_named, ranges_for, uop_config, RangeMode};
use nord_sharc::runner::uop_report::build_summary;
use nord_sharc::runner::uop_report::measure::{
    measure, synthetic_image, synthetic_preset, Measured,
};

const USAGE: &str = "usage: uop-report [--passes N] [--warmup N] [--label TEXT] \
[--engine uop|uop-checked|uop-native] [--uop-families BITS] [--ranges extent|probe] [--image FILE --preset NAME [--msgs FILE]]";

/// Where the core comes from.
#[derive(Debug, PartialEq, Eq)]
enum Source {
    /// The synthetic poll program.
    Synthetic,
    /// The user's own raw core SRAM image, its preset name and an optional message file.
    Firmware {
        image: String,
        preset: String,
        msgs: Option<String>,
    },
}

#[derive(Debug, PartialEq, Eq)]
struct Args {
    source: Source,
    passes: u32,
    warmup: u32,
    engine: Engine,
    engine_name: String,
    families: u32,
    ranges: RangeMode,
    label: Option<String>,
}

/// The arguments of the command line, or the reason they are not usable.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args {
        source: Source::Synthetic,
        passes: 100,
        warmup: 2,
        engine: Engine::Uop,
        engine_name: "uop".into(),
        families: 0,
        ranges: RangeMode::Extent,
        label: None,
    };
    let (mut image, mut preset, mut msgs) = (None, None, None);
    let mut args = args;
    while let Some(flag) = args.next() {
        let mut val = || args.next().ok_or(format!("{flag} needs a value\n{USAGE}"));
        let count = |v: String, what: &str| v.parse::<u32>().map_err(|_| format!("bad {what} {v}"));
        match flag.as_str() {
            "--passes" => a.passes = count(val()?, "--passes")?,
            "--warmup" => a.warmup = count(val()?, "--warmup")?,
            "--label" => a.label = Some(val()?),
            "--engine" => {
                let name = val()?;
                match engine_named(&name) {
                    Some(e) if uop_config(e, 0).is_some() => (a.engine, a.engine_name) = (e, name),
                    _ => {
                        return Err(format!(
                            "--engine {name:?}: want uop, uop-checked or uop-native"
                        ))
                    }
                }
            }
            "--uop-families" => {
                let v = val()?;
                a.families = parse_number(&v).ok_or(format!("bad --uop-families {v}"))?;
            }
            "--ranges" => {
                let v = val()?;
                a.ranges =
                    RangeMode::named(&v).ok_or(format!("--ranges {v:?}: want extent or probe"))?;
            }
            "--image" => image = Some(val()?),
            "--preset" => preset = Some(val()?),
            "--msgs" => msgs = Some(val()?),
            _ => return Err(USAGE.into()),
        }
    }
    a.source = match (image, preset, msgs) {
        (None, None, None) => Source::Synthetic,
        (Some(image), Some(preset), msgs) => Source::Firmware {
            image,
            preset,
            msgs,
        },
        _ => {
            return Err(format!(
                "--image and --preset go together; --msgs needs both\n{USAGE}"
            ))
        }
    };
    Ok(a)
}

/// The image, preset and messages of the source.
fn load(source: &Source) -> Result<(Vec<u8>, Preset, Vec<Msg>), String> {
    match source {
        Source::Synthetic => Ok((synthetic_image(), synthetic_preset(), Vec::new())),
        Source::Firmware {
            image,
            preset,
            msgs,
        } => {
            let data = std::fs::read(image).map_err(|e| format!("{image}: {e}"))?;
            let preset = preset_named(preset).ok_or_else(|| format!("preset {preset:?}"))?;
            let msgs = match msgs {
                Some(f) => {
                    parse_msg_text(&std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?)?
                }
                None => Vec::new(),
            };
            Ok((data, preset, msgs))
        }
    }
}

/// What a run produced: the measured counters and a note for stderr.
struct Run {
    measured: Measured,
    note: String,
}

/// Boots, arms and measures.
fn run(a: &Args) -> Result<Run, String> {
    let (image, preset, msgs) = load(&a.source)?;
    let boot = || boot_core(&image, &preset, &PeriphOpts::default());
    let ranges = ranges_for(a.ranges, boot, &preset, &msgs, a.warmup + a.passes)?;
    let mut core = boot()?;
    let build = arm_for(
        &mut core,
        a.engine,
        a.families,
        preset.idle_pc,
        ranges.as_deref(),
    )?
    .ok_or("the engine does not use the uop path")?;
    let label = a.label.clone().unwrap_or_else(|| match &a.source {
        Source::Synthetic => format!("synthetic poll {}", a.engine_name),
        Source::Firmware { preset, .. } => format!("{preset} {}", a.engine_name),
    });
    let mut job = PassJob::from_preset(&label, core, &preset, msgs, a.engine);
    let measured = measure(&mut job, &label, a.warmup, a.passes)?;
    let mut note = format!("{}; {}", build_summary(&build), measured.report.summary());
    if a.engine == Engine::UopChecked {
        let (c, k) = (measured.checked, measured.skipped);
        note += &format!("; compared with the interpreter: {c} passes, {k} not comparable");
    }
    Ok(Run { measured, note })
}

/// Why the counters of a run are not a steady state or not a check, if they are not.
fn unsound(m: &Measured) -> Option<String> {
    if !m.all_idle {
        return Some("a pass did not return to its poll loop; not a steady state".into());
    }
    (m.skipped > 0).then(|| {
        format!(
            "{} of {} passes could not be compared with the interpreter (the core has peripheral \
             models); this run shows no exactness",
            m.skipped,
            m.checked + m.skipped
        )
    })
}

fn main() {
    let outcome = parse_args(std::env::args().skip(1)).and_then(|a| run(&a));
    match outcome {
        Ok(Run { measured, note }) => {
            println!("{}", measured.report.to_json());
            eprintln!("uop-report: {note}");
            if let Some(e) = &measured.msg_error {
                eprintln!("uop-report: message not delivered: {e}");
            }
            if let Some(why) = unsound(&measured) {
                eprintln!("uop-report: {why}");
                std::process::exit(3);
            }
        }
        Err(e) => {
            eprintln!("uop-report: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn args(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn synthetic(passes: u32, engine: &str) -> Args {
        parse_args(args(&["--passes", &passes.to_string(), "--engine", engine])).unwrap()
    }

    #[test]
    fn arguments_default_parse_and_reject_what_is_not_usable() {
        let d = parse_args(args(&[])).unwrap();
        assert_eq!(
            (d.source, d.passes, d.warmup, d.engine, d.families, d.label),
            (Source::Synthetic, 100, 2, Engine::Uop, 0, None)
        );
        assert_eq!(d.ranges, RangeMode::Extent);
        assert_eq!(
            parse_args(args(&["--ranges", "probe"])).unwrap().ranges,
            RangeMode::Probe
        );
        let a = parse_args(args(&[
            "--passes",
            "50",
            "--warmup",
            "0",
            "--label",
            "x",
            "--engine",
            "uop-checked",
            "--uop-families",
            "0x6",
            "--image",
            "i.bin",
            "--preset",
            "org",
            "--msgs",
            "m.txt",
        ]))
        .unwrap();
        assert_eq!(
            (a.passes, a.warmup, a.engine, a.families),
            (50, 0, Engine::UopChecked, 6)
        );
        assert_eq!(a.label.as_deref(), Some("x"));
        assert_eq!(
            a.source,
            Source::Firmware {
                image: "i.bin".into(),
                preset: "org".into(),
                msgs: Some("m.txt".into())
            }
        );
        for bad in [
            &["--passes", "many"][..],
            &["--passes"],
            &["--bogus"],
            &["--engine", "interp"], // not a uop engine: nothing to report
            &["--engine", "aot"],
            &["--engine", "turbo"],
            &["--uop-families", "x"],
            &["--ranges", "all"],
            &["--ranges"],
            &["--image", "i.bin"], // needs a preset
            &["--preset", "org"],
            &["--msgs", "m.txt"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_synthetic_report_is_valid_json_with_every_step_accounted_once() {
        let r = run(&synthetic(7, "uop")).unwrap();
        assert!(r.measured.all_idle);
        let v: Value = serde_json::from_str(&r.measured.report.to_json()).unwrap();
        assert_eq!(v["schema"], 1);
        let s = &v["stats"];
        assert_eq!(s["passes"], 7);
        let (uop, core_step) = (
            s["steps_uop"].as_u64().unwrap(),
            s["steps_core_step"].as_u64().unwrap(),
        );
        assert_eq!(
            uop + core_step,
            7 * 11,
            "7 passes of the 11-step synthetic pass"
        );
        assert_eq!(s["exits"].as_array().unwrap().len(), 6);
        assert_eq!(s["guard_fail"].as_array().unwrap().len(), 9);
        assert!(v["model"]["tier1_cycles"].as_u64().unwrap() > 0);
        assert!(
            r.note.contains("built") && r.note.contains("passes 7"),
            "{}",
            r.note
        );
    }

    #[test]
    fn probed_ranges_report_the_same_steps_and_a_probe_of_the_run() {
        let probed = |ranges| Args {
            ranges,
            ..synthetic(6, "uop-checked")
        };
        let (a, b) = (
            run(&probed(RangeMode::Extent)),
            run(&probed(RangeMode::Probe)),
        );
        let (a, b) = (a.unwrap().measured, b.unwrap().measured);
        assert_eq!((a.all_idle, a.steps), (b.all_idle, b.steps));
        assert_eq!(a.report.stats.passes, b.report.stats.passes);
    }

    #[test]
    fn the_report_is_deterministic() {
        let a = run(&synthetic(5, "uop")).unwrap().measured.report.to_json();
        assert_eq!(
            a,
            run(&synthetic(5, "uop")).unwrap().measured.report.to_json()
        );
    }

    #[test]
    fn the_label_names_the_run_unless_given() {
        let r = run(&synthetic(1, "uop-checked")).unwrap();
        assert_eq!(r.measured.report.label, "synthetic poll uop-checked");
        let mut a = synthetic(1, "uop");
        a.label = Some("mine".into());
        assert_eq!(run(&a).unwrap().measured.report.label, "mine");
    }

    #[test]
    fn the_checked_engine_reports_the_same_passes_and_how_many_it_compared() {
        let r = run(&synthetic(3, "uop-checked")).unwrap();
        assert_eq!((r.measured.all_idle, r.measured.steps), (true, 33));
        assert_eq!(r.measured.report.stats.passes, 3);
        assert_eq!(
            (r.measured.checked, r.measured.skipped),
            (3, 0),
            "the 3 measured passes, not the 2 of the warm-up"
        );
        assert_eq!(r.measured.report.stats.shadow_checks, 3);
        assert!(r.note.contains("3 passes, 0 not comparable"), "{}", r.note);
        assert_eq!(unsound(&r.measured), None);
        let plain = run(&synthetic(3, "uop")).unwrap();
        assert!(!plain.note.contains("compared"), "{}", plain.note);
    }

    #[test]
    fn a_run_is_unsound_when_a_pass_missed_its_poll_loop_or_a_check_could_not_compare() {
        let ok = run(&synthetic(2, "uop")).unwrap().measured;
        let idle = Measured {
            all_idle: false,
            ..ok.clone()
        };
        assert!(unsound(&idle).unwrap().contains("poll loop"));
        let skipped = Measured {
            checked: 3,
            skipped: 2,
            ..ok
        };
        let why = unsound(&skipped).unwrap();
        assert!(
            why.contains("2 of 5 passes") && why.contains("no exactness"),
            "{why}"
        );
    }

    #[test]
    fn the_native_engine_either_runs_or_says_how_to_enable_it() {
        match run(&synthetic(2, "uop-native")) {
            Ok(r) => assert!(r.measured.all_idle),
            Err(e) => assert!(e.contains("uop-native") && e.contains("--features"), "{e}"),
        }
    }

    #[test]
    fn a_missing_image_or_preset_is_an_error_not_a_panic() {
        let a = |image: &str, preset: &str| Args {
            source: Source::Firmware {
                image: image.into(),
                preset: preset.into(),
                msgs: None,
            },
            ..synthetic(1, "uop")
        };
        assert!(run(&a("no/such/image.bin", "org")).is_err());
        assert!(load(&Source::Firmware {
            image: "x".into(),
            preset: "nope".into(),
            msgs: None
        })
        .is_err());
        assert!(
            run(&Args {
                passes: 0,
                ..synthetic(1, "uop")
            })
            .is_err(),
            "no measured pass"
        );
    }

    #[test]
    fn a_report_prints_as_one_json_object_on_one_line() {
        let j = run(&synthetic(2, "uop")).unwrap().measured.report.to_json();
        assert!(!j.contains('\n') && j.starts_with('{') && j.ends_with('}'));
    }
}
