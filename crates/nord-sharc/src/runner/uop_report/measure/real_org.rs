//! The real ORG Core2 pass under the engine, when the local firmware image is there.
//!
//! OWNER: W1-RUN. The image is the user's own raw core SRAM, never committed; the test skips
//! cleanly without it (`NORD_SHARC_ORG_IMAGE=path/to/core2_sram.bin`).
#[cfg(test)]
mod tests {
    use super::super::measure;
    use crate::runner::live_presets::{boot_core, preset_named};
    use crate::runner::msg::{parse_msg, Msg};
    use crate::runner::pass_sched::{Engine, PassJob};
    use crate::runner::periph_run::PeriphOpts;
    use crate::runner::uop_pass::{arm_for, ranges_for, RangeMode};

    /// The real ORG Core2 pass with eight voices, under the engine as it is today. G2 wants no
    /// store on a translated page and no maintenance in a steady pass, which only holds when the
    /// translated ranges are the pcs the pass runs, not the extent.
    #[test]
    fn the_real_org_pass_is_exact_and_stores_on_no_translated_page_when_the_ranges_are_probed() {
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
        assert!(
            m.report.block_steps.is_some(),
            "a checked run is profiled too"
        );
    }
}
