//! SHARC contenders of `lle-gates`: one core, one engine, one run = boot, arm, warm-up passes and
//! measured passes, every pass timed. The booting and arming are `lle_sharc_bench`'s `core_job`,
//! the pass is `PassJob::run_pass` (the code the scheduler runs), the exactness run is
//! `runner::uop_report::measure`; only the timed loop is new, because neither existing loop
//! returns a time per pass.

use crate::core_job::{job, Arming, CoreSpec};
use crate::sched::{Contender, Sample};
use nord_sharc::core::UopStats;
use nord_sharc::cpu_clock::ThreadClock;
use nord_sharc::runner::live_presets::boot_core;
use nord_sharc::runner::pass_sched::{Engine, PassJob};
use nord_sharc::runner::periph_run::PeriphOpts;
use nord_sharc::runner::uop_pass::{arm_for, RangeMode};
use nord_sharc::runner::uop_report::measure::{measure, synthetic_image, synthetic_preset};
use nord_sharc::runner::uop_report::UopReport;

/// Where a core comes from.
#[derive(Clone, Debug)]
pub enum Source {
    /// The poll program of `uop-report` (no firmware): the wiring check and the smoke test.
    Synthetic,
    /// The user's raw core SRAM image, its preset (`pno`, `org`) and its AOT tag, if any.
    Image {
        image: String,
        preset: String,
        tag: Option<String>,
    },
}

/// One engine of the comparison and its command-line name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineSel {
    pub name: &'static str,
    pub engine: Engine,
}

pub const ENGINES: [EngineSel; 5] = [
    EngineSel {
        name: "interp",
        engine: Engine::Interp,
    },
    EngineSel {
        name: "aot",
        engine: Engine::Aot,
    },
    EngineSel {
        name: "uop",
        engine: Engine::Uop,
    },
    EngineSel {
        name: "uop-checked",
        engine: Engine::UopChecked,
    },
    EngineSel {
        name: "uop-native",
        engine: Engine::UopNative,
    },
];

/// Booted, armed job of `source` for `engine`; an `Err` is the reason the engine is unavailable.
pub fn make_job(
    source: &Source,
    sel: EngineSel,
    ranges: RangeMode,
    passes: u32,
    warmup: u32,
) -> Result<PassJob, String> {
    match source {
        Source::Image { image, preset, tag } => {
            if sel.engine == Engine::Aot && tag.is_none() {
                return Err(
                    "needs an AOT tag (--aot-tag-*, NORD_SHARC_AOT_HOT at build time)".into(),
                );
            }
            let arming = Arming {
                families: 0,
                ranges,
                probe_passes: warmup + passes,
            };
            let spec = CoreSpec {
                name: preset.to_uppercase(),
                image: image.clone(),
                preset: preset.clone(),
                msgs: None,
                tag: tag.clone(),
            };
            Ok(job(&spec, sel.engine, arming)?.job)
        }
        Source::Synthetic => {
            if sel.engine == Engine::Aot {
                return Err("the synthetic core has no AOT module".into());
            }
            let preset = synthetic_preset();
            let mut core = boot_core(&synthetic_image(), &preset, &PeriphOpts::default())?;
            arm_for(&mut core, sel.engine, 0, preset.idle_pc, None)?;
            Ok(PassJob::from_preset(
                sel.name,
                core,
                &preset,
                Vec::new(),
                sel.engine,
            ))
        }
    }
}

/// Times `passes` passes after `warmup`; the engine counters are of the measured passes only.
pub fn timed_run(
    job: &mut PassJob,
    label: &str,
    warmup: u32,
    passes: u32,
) -> Result<Sample, String> {
    job.msgs.sort_by_key(|m| m.pass);
    let (mut next, mut steady) = (0usize, true);
    let (mut us, mut cpu_us) = (Vec::with_capacity(passes as usize), Vec::new());
    for p in 0..warmup + passes {
        if p == warmup {
            job.core.uop_reset_stats();
        }
        job.before_pass(p, &mut next)?;
        let clock = ThreadClock::start();
        let (_, idle) = job.run_pass();
        let (wall, cpu) = (clock.wall_ns(), clock.elapsed_ns());
        steady &= idle;
        if p >= warmup {
            us.push(wall as f64 / 1e3);
            cpu_us.push(cpu as f64 / 1e3);
        }
    }
    let counters = (job.engine != Engine::Interp && job.engine != Engine::Aot)
        .then(|| UopReport::measured(label, &job.core.uop_stats(), &UopStats::default()));
    Ok(Sample {
        us,
        cpu_us,
        counters,
        steady,
    })
}

/// One contender: a core on an engine. Unavailable engines fail on their first run with the reason.
pub fn contender(
    source: Source,
    sel: EngineSel,
    ranges: RangeMode,
    passes: u32,
    warmup: u32,
) -> Contender {
    let checked = sel.engine == Engine::UopChecked;
    Contender {
        label: sel.name.into(),
        timed: !checked,
        run: Box::new(move || {
            let mut j = make_job(&source, sel, ranges, passes, warmup)?;
            if checked {
                // the checked engine allocates: its run shows exactness and counters, never a time
                let m = measure(&mut j, sel.name, warmup, passes)?;
                return Ok(Sample {
                    counters: Some(m.report),
                    steady: m.all_idle && m.skipped == 0,
                    ..Sample::default()
                });
            }
            timed_run(&mut j, sel.name, warmup, passes)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sched::alternate;
    use nord_sharc::runner::uop_report::measure::SYNTH_PASS_STEPS;

    fn sel(name: &str) -> EngineSel {
        *ENGINES.iter().find(|e| e.name == name).unwrap()
    }

    #[test]
    fn a_synthetic_uop_run_times_every_measured_pass_and_counts_only_those() {
        let mut j = make_job(&Source::Synthetic, sel("uop"), RangeMode::Extent, 6, 2).unwrap();
        let s = timed_run(&mut j, "uop", 2, 6).unwrap();
        assert_eq!((s.us.len(), s.cpu_us.len()), (6, 6));
        assert!(s.steady && s.us.iter().all(|u| *u > 0.0));
        let c = s.counters.unwrap();
        assert_eq!(
            c.stats.passes, 6,
            "the warm-up passes are not in the counters"
        );
        assert_eq!(
            c.stats.steps_uop + c.stats.steps_core_step,
            6 * SYNTH_PASS_STEPS
        );
        assert_eq!(c.window.name(), "measured");
    }

    #[test]
    fn the_interpreter_has_no_counters_and_aot_needs_a_module() {
        let mut j = make_job(&Source::Synthetic, sel("interp"), RangeMode::Extent, 3, 1).unwrap();
        let s = timed_run(&mut j, "interp", 1, 3).unwrap();
        assert!(s.counters.is_none() && s.steady);
        let e = make_job(&Source::Synthetic, sel("aot"), RangeMode::Extent, 3, 1).err();
        assert!(e.unwrap().contains("no AOT module"));
    }

    #[test]
    fn the_checked_engine_compares_and_is_not_a_timing() {
        let mut c = contender(
            Source::Synthetic,
            sel("uop-checked"),
            RangeMode::Extent,
            4,
            1,
        );
        assert!(!c.timed);
        let s = (c.run)().unwrap();
        assert!(s.us.is_empty(), "no time is taken on the allocating engine");
        assert!(
            s.counters.unwrap().replay.checked > 0,
            "passes were compared"
        );
    }

    #[test]
    fn the_engines_alternate_on_the_synthetic_core_without_firmware() {
        let mut cs: Vec<Contender> = ENGINES
            .iter()
            .map(|e| contender(Source::Synthetic, *e, RangeMode::Extent, 5, 1))
            .collect();
        let m = alternate(&mut cs, 2, &|| Some(1.0));
        for s in &m.series {
            match s.label.as_str() {
                "aot" => assert!(s.error.as_deref().unwrap().contains("no AOT module")),
                "uop-native" if s.error.is_some() => {
                    assert!(
                        s.error.as_deref().unwrap().contains("uop-native"),
                        "{:?}",
                        s.error
                    )
                }
                label => {
                    assert_eq!(s.runs.len(), 2, "{label}: {:?}", s.error);
                    assert!(s.runs.iter().all(|r| r.steady), "{label}");
                }
            }
        }
    }
}
