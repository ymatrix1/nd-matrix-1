//! The run behind `uop-report`: load the core, arm the engine, measure, and say whether the
//! counters are a steady state and a check.
use crate::args::{Args, Source};
use nord_sharc::runner::live_presets::{boot_core, preset_named, Preset};
use nord_sharc::runner::msg::{parse_msg_text, Msg};
use nord_sharc::runner::pass_sched::{Engine, PassJob};
use nord_sharc::runner::periph_run::PeriphOpts;
use nord_sharc::runner::uop_pass::{arm_for, ranges_for};
use nord_sharc::runner::uop_report::build_summary;
use nord_sharc::runner::uop_report::measure::{
    measure, synthetic_image, synthetic_preset, Measured,
};

/// The image, preset and messages of the source.
pub fn load(source: &Source) -> Result<(Vec<u8>, Preset, Vec<Msg>), String> {
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

/// What a run produced: the measured counters and the notes for stderr.
pub struct Run {
    pub measured: Measured,
    /// The build line, the summary of the counters and, for the checked engine, how many passes
    /// were compared.
    pub note: String,
    /// The block-length histogram of the measured passes, as one line.
    pub histogram: Option<String>,
}

/// Boots, arms and measures.
pub fn run(a: &Args) -> Result<Run, String> {
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
    let histogram = measured.report.histogram_text();
    Ok(Run {
        measured,
        note,
        histogram,
    })
}

/// Why the counters of a run are not a steady state or not a check, if they are not.
pub fn unsound(m: &Measured) -> Option<String> {
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

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::args::{parse_args, tests::args};
    use nord_sharc::runner::uop_pass::RangeMode;
    use serde_json::Value;

    pub fn synthetic(passes: u32, engine: &str) -> Args {
        parse_args(args(&["--passes", &passes.to_string(), "--engine", engine])).unwrap()
    }

    #[test]
    fn the_synthetic_report_is_valid_json_with_every_step_accounted_once() {
        let r = run(&synthetic(7, "uop")).unwrap();
        assert!(r.measured.all_idle);
        let v: Value = serde_json::from_str(&r.measured.report.to_json()).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(
            v["window"], "measured",
            "the counters are the measured passes"
        );
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
    fn the_report_carries_the_block_length_histogram_in_the_json_and_in_the_notes() {
        let r = run(&synthetic(6, "uop")).unwrap();
        let v: Value = serde_json::from_str(&r.measured.report.to_json()).unwrap();
        let h = &v["block_steps"];
        assert_eq!(h["edges"].as_array().unwrap().len(), 11);
        assert_eq!(h["counts"].as_array().unwrap().len(), 12);
        // `entries + empty + 2 * chained <= blocks_run` (a chained entry ran at least two
        // linked blocks), with equality when nothing chained (`hist.rs`).
        let (entries, empty, chained) = (
            h["entries"].as_u64().unwrap(),
            h["empty"].as_u64().unwrap(),
            h["chained"].as_u64().unwrap(),
        );
        let blocks = v["stats"]["blocks_run"].as_u64().unwrap();
        assert!(
            entries + empty + 2 * chained <= blocks,
            "{entries} + {empty} + 2 * {chained} <= {blocks}"
        );
        assert!(
            entries + empty + chained > 0,
            "the table counts the block entries"
        );
        if chained == 0 {
            assert_eq!(entries + empty, blocks, "one table entry per block entered");
        }
        let line = r.histogram.expect("a uop engine is profiled");
        assert!(line.starts_with("steps per block entry ["), "{line}");
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
        let v: Value = serde_json::from_str(&r.measured.report.to_json()).unwrap();
        assert_eq!(v["replay"]["checked"], 3);
        assert_eq!(v["replay"]["skipped"], 0);
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
