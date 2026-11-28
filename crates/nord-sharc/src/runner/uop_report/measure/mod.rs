//! The single-thread measuring loop behind `uop-report`: boot, arm, run `warmup + passes` passes
//! of a [`PassJob`] and read the counters of the measured passes. No threads and no clocks, so two
//! runs give the same report. The synthetic poll program (no firmware) the tool and the tests use
//! as the wiring check is in [`synthetic`].
//!
//! OWNER: W1-RUN. The threaded scheduler of `lle_sharc_bench` (`pass_sched::run_ticks`) cannot
//! reset the counters between the phases (the cores run on their own threads); it snapshots them
//! at the first measured pass instead (`CoreSeries::uop_at_warmup`) and the bench reports the
//! difference (`UopReport::measured`). This loop resets the engine counters at the first measured
//! pass, which is what the steady-state gates (`maintain_calls == 0`, `smc == 0`) need. Message
//! delivery, ping-pong and FX drive are `PassJob::before_pass`, the same code the scheduler runs.
//! The uop engines run through the entry-watching pass loops of `uop_pass`, which move no counter,
//! to fill the block-length histogram ([`BlockHistogram`]).
#[cfg(test)]
mod real_org;
mod synthetic;

pub use synthetic::{
    synthetic_image, synthetic_preset, SYNTH_CODE, SYNTH_PASS_STEPS, SYNTH_POLL_PC, SYNTH_TRIGGER,
};

use super::{BlockHistogram, Replay, UopReport, Window};
use crate::core::Shadow;
use crate::runner::pass_sched::{Engine, PassJob};
use crate::runner::uop_pass::{lean_pass_uop_entries, lean_pass_uop_shadowed_entries};

/// What [`measure`] observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measured {
    /// The engine counters of the measured passes (the warm-up is not in them), with the replay
    /// counts of the checked engine and, for the uop engines, the block-length histogram.
    pub report: UopReport,
    /// Every pass, warm-up included, returned to its poll loop and every message was delivered.
    pub all_idle: bool,
    /// Steps of the measured passes (what the interpreter would have run).
    pub steps: u64,
    /// The first message that could not be delivered, if any.
    pub msg_error: Option<String>,
    /// Measured passes of the `uop-checked` engine that were replayed on the interpreter and found
    /// equal to it (also `report.replay.checked`). 0 for every other engine.
    pub checked: u32,
    /// Measured passes of the `uop-checked` engine that could not be replayed (the core has
    /// peripheral models) and so were not compared (also `report.replay.skipped`): a run with
    /// `skipped > 0` has not shown exactness.
    pub skipped: u32,
}

/// Runs `warmup + passes` passes of `job` on this thread. The engine counters are zeroed when the
/// first measured pass begins, so `report` holds only measured passes (and `translations` counts
/// what was retranslated during them, 0 in a steady state). `job` must already be armed for its
/// engine (`uop_pass::arm_for`). With `Engine::UopChecked` every pass, warm-up included, is replayed
/// on the interpreter and a difference is an error naming the pass, not a panic; the measured passes
/// are counted in [`Measured::checked`] and `skipped` and in the report's own [`Replay`] fields
/// (`stats.shadow_checks` stays what the engine counted: blocks checked). The uop engines also
/// fill the histogram of steps per block entry, over the measured passes.
pub fn measure(
    job: &mut PassJob,
    label: &str,
    warmup: u32,
    passes: u32,
) -> Result<Measured, String> {
    if passes == 0 {
        return Err("no measured passes (--passes 0)".into());
    }
    job.msgs.sort_by_key(|m| m.pass);
    let (mut next_msg, mut all_idle, mut steps, mut msg_error) = (0, true, 0, None);
    let (mut checked, mut skipped) = (0, 0);
    let mut hist = BlockHistogram::new();
    for p in 0..u64::from(warmup) + u64::from(passes) {
        let counted = p >= u64::from(warmup);
        if p == u64::from(warmup) {
            job.core.uop_reset_stats();
        }
        if let Err(e) = job.before_pass(p as u32, &mut next_msg) {
            all_idle = false;
            msg_error.get_or_insert(e);
        }
        let mut on_entry = |e| {
            if counted {
                hist.add_entry(&e);
            }
        };
        let (n, idle) = match job.engine {
            Engine::Uop | Engine::UopNative => {
                lean_pass_uop_entries(&mut job.core, &job.cfg, &mut on_entry)
            }
            Engine::UopChecked => {
                let (n, idle, verdict) =
                    lean_pass_uop_shadowed_entries(&mut job.core, &job.cfg, &mut on_entry);
                match verdict {
                    Shadow::Match => checked += u32::from(counted),
                    Shadow::Skipped => skipped += u32::from(counted),
                    Shadow::Mismatch(d) => {
                        return Err(format!("pass {p} differs from the interpreter: {d}"))
                    }
                }
                (n, idle)
            }
            Engine::Interp | Engine::Aot => job.run_pass(),
        };
        all_idle &= idle;
        if counted {
            steps += n;
        }
    }
    let profiled = matches!(
        job.engine,
        Engine::Uop | Engine::UopChecked | Engine::UopNative
    );
    let report = UopReport::new(label, job.core.uop_stats(), Window::Measured)
        .with_replay(Replay {
            checked: u64::from(checked),
            skipped: u64::from(skipped),
        })
        .with_block_steps(profiled.then_some(hist));
    Ok(Measured {
        report,
        all_idle,
        steps,
        msg_error,
        checked,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::UopStats;
    use crate::runner::live_presets::boot_core;
    use crate::runner::msg::Msg;
    use crate::runner::periph_run::PeriphOpts;
    use crate::runner::uop_pass::{arm_for, ranges_for, RangeMode};

    /// The synthetic job on `engine`, armed.
    fn job(engine: Engine) -> PassJob {
        let preset = synthetic_preset();
        let mut core = boot_core(&synthetic_image(), &preset, &PeriphOpts::default()).unwrap();
        arm_for(&mut core, engine, 0, preset.idle_pc, None).unwrap();
        PassJob::from_preset("synthetic", core, &preset, Vec::new(), engine)
    }

    #[test]
    fn probed_ranges_measure_the_same_passes_as_the_extent_scan() {
        let preset = synthetic_preset();
        let boot = || boot_core(&synthetic_image(), &preset, &PeriphOpts::default());
        let ranges = ranges_for(RangeMode::Probe, boot, &preset, &[], 6).unwrap();
        let mut core = boot().unwrap();
        arm_for(&mut core, Engine::Uop, 0, preset.idle_pc, ranges.as_deref()).unwrap();
        let mut probed = PassJob::from_preset("p", core, &preset, vec![], Engine::Uop);
        let a = measure(&mut probed, "same", 2, 4).unwrap();
        let b = measure(&mut job(Engine::Uop), "same", 2, 4).unwrap();
        assert_eq!((a.all_idle, a.steps), (b.all_idle, b.steps));
        assert_eq!(a.report.stats.passes, b.report.stats.passes);
        assert_eq!(
            a.report.stats.steps_uop + a.report.stats.steps_core_step,
            a.steps,
            "every probed step is accounted once"
        );
    }

    #[test]
    fn only_the_measured_passes_are_in_the_report() {
        let mut j = job(Engine::Uop);
        let m = measure(&mut j, "synthetic", 3, 5).unwrap();
        let s = m.report.stats;
        assert_eq!((m.all_idle, m.msg_error), (true, None));
        assert_eq!(m.steps, 5 * SYNTH_PASS_STEPS);
        assert_eq!(s.passes, 5, "the 3 warm-up passes were reset away");
        assert_eq!(
            s.steps_uop + s.steps_core_step,
            m.steps,
            "every measured step is accounted once"
        );
        assert_eq!((s.maintain_calls, s.smc, s.translations), (0, 0, 0));
        assert_eq!(m.report.label, "synthetic");
        assert_eq!(m.report.window, Window::Measured);
        // the core itself ran all 8 passes
        assert_eq!(j.core.steps, 8 * SYNTH_PASS_STEPS);
    }

    #[test]
    fn without_a_warm_up_every_pass_is_measured() {
        let mut j = job(Engine::Uop);
        let m = measure(&mut j, "w0", 0, 4).unwrap();
        assert_eq!(m.report.stats.passes, 4);
        assert_eq!(m.steps, 4 * SYNTH_PASS_STEPS);
    }

    #[test]
    fn the_measurement_is_deterministic() {
        let a = measure(&mut job(Engine::Uop), "d", 2, 6).unwrap();
        let b = measure(&mut job(Engine::Uop), "d", 2, 6).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.report.to_json(), b.report.to_json());
    }

    #[test]
    fn the_checked_engine_measures_the_same_passes_and_finds_no_difference() {
        let mut j = job(Engine::UopChecked);
        let m = measure(&mut j, "checked", 1, 3).unwrap();
        assert_eq!((m.all_idle, m.steps), (true, 3 * SYNTH_PASS_STEPS));
        assert_eq!(m.report.stats.passes, 3);
        assert_eq!(
            (m.checked, m.skipped),
            (3, 0),
            "the three measured passes were replayed on the interpreter"
        );
        assert_eq!(
            m.report.replay,
            Replay {
                checked: 3,
                skipped: 0
            },
            "and counted in the report, in fields of their own"
        );
        let s = m.report.stats;
        assert_eq!(
            (s.shadow_checks, s.shadow_skipped, s.shadow_mismatch),
            (0, 0, 0),
            "`stats.shadow_*` keep the engine's meaning (blocks checked): pass counts are not in it"
        );
    }

    #[test]
    fn a_checked_run_on_a_core_that_cannot_be_replayed_says_so_instead_of_claiming_a_match() {
        let preset = synthetic_preset();
        let mut core = boot_core(&synthetic_image(), &preset, &PeriphOpts::default()).unwrap();
        core.enable_epdma(); // a peripheral model: the passes cannot be rewound
        arm_for(&mut core, Engine::UopChecked, 0, preset.idle_pc, None).unwrap();
        let mut j = PassJob::from_preset("p", core, &preset, vec![], Engine::UopChecked);
        let m = measure(&mut j, "periph", 1, 2).unwrap();
        assert_eq!((m.checked, m.skipped), (0, 2));
        assert_eq!(
            m.report.replay,
            Replay {
                checked: 0,
                skipped: 2
            }
        );
        assert_eq!(
            m.report.stats.shadow_skipped, 0,
            "the engine skipped no block"
        );
        assert_eq!(
            (m.all_idle, m.steps),
            (true, 2 * SYNTH_PASS_STEPS),
            "the passes still ran"
        );
        // the engines that are not checked count neither
        let m = measure(&mut job(Engine::Uop), "plain", 1, 2).unwrap();
        assert_eq!((m.checked, m.skipped), (0, 0));
        assert_eq!(m.report.replay, Replay::default());
    }

    #[test]
    fn an_engine_that_is_not_armed_counts_nothing_but_still_runs_the_passes() {
        let preset = synthetic_preset();
        let core = boot_core(&synthetic_image(), &preset, &PeriphOpts::default()).unwrap();
        let mut j = PassJob::from_preset("i", core, &preset, vec![], Engine::Interp);
        let m = measure(&mut j, "interp", 1, 4).unwrap();
        assert_eq!(m.report.stats, UopStats::default());
        assert_eq!((m.all_idle, m.steps), (true, 4 * SYNTH_PASS_STEPS));
        assert_eq!(
            m.report.block_steps, None,
            "the interpreter has no blocks to profile"
        );
    }

    #[test]
    fn messages_are_delivered_before_the_pass_they_name_in_pass_order() {
        let preset = synthetic_preset();
        let core = boot_core(&synthetic_image(), &preset, &PeriphOpts::default()).unwrap();
        let msgs = vec![
            Msg {
                pass: 2,
                word1: 0xA_0102,
                word2: 0x22,
            }, // not in pass order on purpose
            Msg {
                pass: 0,
                word1: 0xA_0100,
                word2: 0x11,
            },
            Msg {
                pass: 99,
                word1: 0xA_0104,
                word2: 0x99,
            }, // after the run: never delivered
        ];
        let mut j = PassJob::from_preset("m", core, &preset, msgs, Engine::Interp);
        let m = measure(&mut j, "msgs", 0, 4).unwrap();
        assert_eq!((m.all_idle, m.msg_error), (true, None));
        assert_eq!(j.core.dm_read(0xA_0100), Ok(0x11));
        assert_eq!(j.core.dm_read(0xA_0102), Ok(0x22));
        assert_eq!(
            j.core.dm_read(0xA_0104),
            Ok(0),
            "a message of a later pass waits"
        );
    }

    #[test]
    fn zero_measured_passes_is_an_error_not_an_empty_report() {
        let mut j = job(Engine::Uop);
        assert!(measure(&mut j, "none", 5, 0).is_err());
        assert_eq!(j.core.steps, 0, "and nothing was run");
    }

    #[test]
    fn the_block_length_histogram_counts_the_measured_entries_only() {
        // 2 warm-up passes, 5 measured: the table holds the entries of the 5, and the entries
        // add up to the engine's own counters
        let m = measure(&mut job(Engine::Uop), "hist", 2, 5).unwrap();
        let h = m.report.block_steps.expect("a uop engine is profiled");
        let s = m.report.stats;
        assert!(
            h.entries() + h.chained() + h.empty() > 0,
            "the engine entered blocks"
        );
        assert_eq!(
            h.entries() + h.chained() + h.empty(),
            s.blocks_run,
            "one table entry per block the driver entered, those that ran no step included"
        );
        if h.chained() == 0 {
            assert_eq!(h.steps(), s.steps_uop, "every counted step is in a bucket");
        }
        assert_eq!(h.counts().iter().sum::<u64>(), h.entries());
        // a longer measurement has proportionally more entries: the warm-up is not in the table
        let more = measure(&mut job(Engine::Uop), "hist", 2, 10).unwrap();
        let hm = more.report.block_steps.unwrap();
        assert_eq!(hm.entries(), 2 * h.entries());
        // and the checked engine fills the same table from its engine run only
        let c = measure(&mut job(Engine::UopChecked), "hist", 2, 5).unwrap();
        assert_eq!(c.report.block_steps.as_ref(), Some(&h));
    }
}
