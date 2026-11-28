//! The single-thread measuring loop behind `uop-report`: boot, arm, run `warmup + passes` passes
//! of a [`PassJob`] and read the counters of the measured passes. No threads and no clocks, so two
//! runs give the same report. Also the synthetic poll program (no firmware) the tool and the
//! tests use as the wiring check.
//!
//! OWNER: W1-RUN. The scheduler of `lle_sharc_bench` (`pass_sched::run_ticks`) has no hook between
//! warm-up and measurement, so its counters cover every pass it ran; this loop resets the engine
//! counters at the first measured pass instead, which is what the steady-state gates (`maintain_calls
//! == 0`, `smc == 0`) need. Message delivery, ping-pong and FX drive are `PassJob::before_pass`,
//! the same code the scheduler runs.
use super::UopReport;
use crate::core::Shadow;
use crate::runner::live_presets::Preset;
use crate::runner::pass_sched::{Engine, PassJob};
use crate::runner::uop_pass::lean_pass_uop_shadowed;

/// Short-word address of the synthetic program (the VISA code window).
pub const SYNTH_CODE: u32 = 0x12_0000;
/// Mailbox address of the synthetic pass: a scripted device word that always reads 0, so a pass
/// ends whenever the pc is back on the poll instruction.
pub const SYNTH_TRIGGER: u32 = 0x3000_0000;
/// The poll instruction of the synthetic program, after its ten NOPs.
pub const SYNTH_POLL_PC: u32 = SYNTH_CODE + 10;
/// Steps of one synthetic pass: the poll jump and the ten NOPs it runs through before landing on
/// the poll instruction again.
pub const SYNTH_PASS_STEPS: u64 = 11;

/// SRAM image of the synthetic program: ten NOPs (`0x0001`) at [`SYNTH_CODE`], then
/// `IF TRUE JUMP SYNTH_CODE` (`0x063e 0x0012 0x0000`) at [`SYNTH_POLL_PC`]. Hand-written parcels,
/// no firmware.
pub fn synthetic_image() -> Vec<u8> {
    let parcels = [
        0x0001u16, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x063e,
        0x0012, 0x0000,
    ];
    let mut image = vec![0u8; SYNTH_CODE as usize * 2 + parcels.len() * 2];
    for (k, p) in parcels.iter().enumerate() {
        let o = (SYNTH_CODE as usize + k) * 2;
        image[o..o + 2].copy_from_slice(&p.to_le_bytes());
    }
    image
}

/// The start-up of the synthetic program: idle on the poll instruction, no boot steps.
pub fn synthetic_preset() -> Preset {
    Preset {
        entry: SYNTH_POLL_PC,
        idle_pc: SYNTH_POLL_PC,
        trigger: SYNTH_TRIGGER,
        boot: 0,
        irq: None,
        ping_pong: None,
        devs: vec![(SYNTH_TRIGGER, 0)],
        flag1: false,
        sy: false,
        smp: false,
        fx: None,
    }
}

/// What [`measure`] observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measured {
    /// The engine counters of the measured passes (the warm-up is not in them).
    pub report: UopReport,
    /// Every pass, warm-up included, returned to its poll loop and every message was delivered.
    pub all_idle: bool,
    /// Steps of the measured passes (what the interpreter would have run).
    pub steps: u64,
    /// The first message that could not be delivered, if any.
    pub msg_error: Option<String>,
    /// Measured passes of the `uop-checked` engine that were replayed on the interpreter and found
    /// equal to it (also `report.stats.shadow_checks`). 0 for every other engine.
    pub checked: u32,
    /// Measured passes of the `uop-checked` engine that could not be replayed (the core has
    /// peripheral models) and so were not compared (also `report.stats.shadow_skipped`): a run with
    /// `skipped > 0` has not shown exactness.
    pub skipped: u32,
}

/// Runs `warmup + passes` passes of `job` on this thread. The engine counters are zeroed when the
/// first measured pass begins, so `report` holds only measured passes (and `translations` counts
/// what was retranslated during them, 0 in a steady state). `job` must already be armed for its
/// engine (`uop_pass::arm_for`). With `Engine::UopChecked` every pass, warm-up included, is replayed
/// on the interpreter and a difference is an error naming the pass, not a panic; the measured passes
/// are counted as `shadow_checks` or `shadow_skipped` of the report (the engine does not count them,
/// `shadow_run_with` leaves that to its caller) and in [`Measured::checked`] and `skipped`.
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
    for p in 0..u64::from(warmup) + u64::from(passes) {
        if p == u64::from(warmup) {
            job.core.uop_reset_stats();
        }
        if let Err(e) = job.before_pass(p as u32, &mut next_msg) {
            all_idle = false;
            msg_error.get_or_insert(e);
        }
        let (n, idle) = if job.engine == Engine::UopChecked {
            let (n, idle, verdict) = lean_pass_uop_shadowed(&mut job.core, &job.cfg);
            let measured = p >= u64::from(warmup);
            match verdict {
                Shadow::Match => checked += u32::from(measured),
                Shadow::Skipped => skipped += u32::from(measured),
                Shadow::Mismatch(d) => {
                    return Err(format!("pass {p} differs from the interpreter: {d}"))
                }
            }
            (n, idle)
        } else {
            job.run_pass()
        };
        all_idle &= idle;
        if p >= u64::from(warmup) {
            steps += n;
        }
    }
    let mut report = UopReport::from_core(label, &job.core);
    report.stats.shadow_checks = u64::from(checked);
    report.stats.shadow_skipped = u64::from(skipped);
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
    use crate::runner::pass_sched::Engine;
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
    fn the_synthetic_program_is_a_pass_of_11_steps_that_ends_on_the_poll_instruction() {
        let preset = synthetic_preset();
        let core = boot_core(&synthetic_image(), &preset, &PeriphOpts::default()).unwrap();
        assert_eq!(core.pc, SYNTH_POLL_PC);
        let mut j = PassJob::from_preset("s", core, &preset, vec![], Engine::Interp);
        assert_eq!(j.run_pass(), (SYNTH_PASS_STEPS, true));
        assert_eq!(j.core.pc, SYNTH_POLL_PC);
        assert_eq!(j.run_pass(), (SYNTH_PASS_STEPS, true), "and again");
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
        let s = m.report.stats;
        assert_eq!(
            (s.shadow_checks, s.shadow_skipped, s.shadow_mismatch),
            (3, 0, 0),
            "and counted in the report"
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
        assert_eq!(m.report.stats.shadow_skipped, 2);
        assert_eq!(
            (m.all_idle, m.steps),
            (true, 2 * SYNTH_PASS_STEPS),
            "the passes still ran"
        );
        // the engines that are not checked count neither
        let m = measure(&mut job(Engine::Uop), "plain", 1, 2).unwrap();
        assert_eq!((m.checked, m.skipped), (0, 0));
    }

    #[test]
    fn an_engine_that_is_not_armed_counts_nothing_but_still_runs_the_passes() {
        let preset = synthetic_preset();
        let core = boot_core(&synthetic_image(), &preset, &PeriphOpts::default()).unwrap();
        let mut j = PassJob::from_preset("i", core, &preset, vec![], Engine::Interp);
        let m = measure(&mut j, "interp", 1, 4).unwrap();
        assert_eq!(m.report.stats, UopStats::default());
        assert_eq!((m.all_idle, m.steps), (true, 4 * SYNTH_PASS_STEPS));
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

    /// The real ORG Core2 pass with eight voices, under the engine as it is today. Needs the local
    /// firmware image (`NORD_SHARC_ORG_IMAGE=path/to/core2_sram.bin`, never committed); skipped
    /// without it. G2 wants no store on a translated page and no maintenance in a steady pass,
    /// which only holds when the translated ranges are the pcs the pass runs, not the extent.
    #[test]
    fn the_real_org_pass_is_exact_and_stores_on_no_translated_page_when_the_ranges_are_probed() {
        use crate::runner::live_presets::preset_named;
        use crate::runner::msg::parse_msg;
        let Some(path) = std::env::var_os("NORD_SHARC_ORG_IMAGE") else {
            eprintln!("skipped: NORD_SHARC_ORG_IMAGE is not set (local firmware image)");
            return;
        };
        let image = std::fs::read(path).unwrap();
        let preset = preset_named("org").unwrap();
        let msgs: Vec<Msg> = (0..8)
            .map(|k| parse_msg(&format!("0:0x{:x}=0x1215fb", 0xc10b8 + k * 0x18)).unwrap())
            .collect();
        let boot = || boot_core(&image, &preset, &PeriphOpts::default());
        let ranges = ranges_for(RangeMode::Probe, boot, &preset, &msgs, 5).unwrap();
        let mut core = boot().unwrap();
        arm_for(
            &mut core,
            Engine::UopChecked,
            0,
            preset.idle_pc,
            ranges.as_deref(),
        )
        .unwrap();
        let mut j = PassJob::from_preset("org", core, &preset, msgs, Engine::UopChecked);
        let m = measure(&mut j, "org", 2, 3).unwrap();
        assert!(m.all_idle && m.msg_error.is_none());
        // an idle ORG pass is 4,266 steps (observed on the PNO core 2 image): real work ran
        assert!(m.steps >= 3 * 4_000, "{} steps in 3 passes", m.steps);
        assert_eq!((m.checked, m.skipped), (3, 0), "compared, not skipped");
        let s = m.report.stats;
        assert_eq!(
            (s.smc, s.maintain_calls, s.translations),
            (0, 0, 0),
            "{s:?}"
        );
        assert_eq!(s.steps_uop + s.steps_core_step, m.steps, "every step once");
    }

    #[test]
    fn zero_measured_passes_is_an_error_not_an_empty_report() {
        let mut j = job(Engine::Uop);
        assert!(measure(&mut j, "none", 5, 0).is_err());
        assert_eq!(j.core.steps, 0, "and nothing was run");
    }
}
