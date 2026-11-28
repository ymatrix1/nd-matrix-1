//! Where the code is, from what a probe run executes: [`pc_ranges`] merges short-word pcs into the
//! ranges `Core::enable_uop` takes, [`probe_ranges`] collects the pcs of a few interpreter passes.
//!
//! OWNER: W1-RUN. The extent scan of [`super::code_ranges`] takes every non-zero parcel for code,
//! so the data tables of a booted core are translated too and every store into them counts as
//! `smc` (G2 wants 0). The probe translates only what the probed states run; code outside stays
//! in the interpreter (exact, slower).
//!
//! ASSUMED that the probe ranges leave no store on a translated page of a real core: the claim
//! rests on one observation that is not reproducible from the repository (a debug-build probe
//! test on the local PNO core 2 image under the org preset, one steady pass, deleted afterwards:
//! the extent scan gave 13 ranges, 75,841 short words, 309 pages and 1,137 of the 1,197 stores of
//! the pass (95%) landed on those pages, the voice tables at `0x1801b8..` being non-zero data;
//! the pcs the pass executed gave 5 ranges, 10,382 words, 46 pages and no store on them). What
//! would confirm it, deterministic counters and no clock: with the user's own core image,
//! `uop-report --image CORE2.bin --preset org --ranges extent` against the same command with
//! `--ranges probe`; compare the `pages` of the build line (stderr) and the `smc` counter of the
//! JSON (stdout), and record both in `docs/research/` (the local-only test
//! `the_real_org_pass_is_exact_and_stores_on_no_translated_page_when_the_ranges_are_probed` in
//! `uop_report/measure` asserts the probe half when `NORD_SHARC_ORG_IMAGE` is set).
use super::arm::{CODE_WINDOW, RANGE_GAP, RANGE_TAIL};
use crate::core::Core;
use crate::runner::live_presets::Preset;
use crate::runner::msg::Msg;
use crate::runner::pass_sched::{Engine, PassJob};
use crate::runner::system::probe_pass;
use std::collections::BTreeSet;

/// How the code ranges of a core are found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeMode {
    /// The extent of the non-zero parcels ([`super::code_ranges`]), the design row's default.
    Extent,
    /// The pcs of probe passes on the interpreter ([`probe_ranges`]).
    Probe,
}

impl RangeMode {
    /// The mode a command-line name selects: `extent` or `probe`.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "extent" => Some(Self::Extent),
            "probe" => Some(Self::Probe),
            _ => None,
        }
    }
}

/// Merges short-word pcs into `(lo, hi)` ranges for `Core::enable_uop`. A range runs from its first
/// pc to [`RANGE_TAIL`] parcels past its last one (clamped to the window); a gap of [`RANGE_GAP`]
/// unlisted parcels or more ends it. Pcs outside [`CODE_WINDOW`] are left out (the engine accepts
/// no range there; they stay in the interpreter). The pcs must ascend.
pub fn pc_ranges(pcs: impl IntoIterator<Item = u32>) -> Vec<(u32, u32)> {
    let end_of = |last: u32| (last + 1 + RANGE_TAIL).min(CODE_WINDOW.end);
    let mut out = Vec::new();
    let mut open: Option<(u32, u32)> = None; // (first, last) pc of the open range
    for pc in pcs.into_iter().filter(|pc| CODE_WINDOW.contains(pc)) {
        open = match open {
            Some((first, last)) if pc - last > RANGE_GAP => {
                out.push((first, end_of(last)));
                Some((pc, pc))
            }
            Some((first, _)) => Some((first, pc)),
            None => Some((pc, pc)),
        };
    }
    out.extend(open.map(|(first, last)| (first, end_of(last))));
    out
}

/// The ranges of the pcs `passes` interpreter passes execute. `core` must be a fresh boot of the
/// core that will be armed and `msgs` its messages: the probe runs the same `before_pass` (ARM
/// messages, ping-pong, FX drive) as the measured run, so it passes through the same states, and
/// the engine is exact, so the measured run executes no pc the probe did not. A message that cannot
/// be delivered is left for the measured run to report.
pub fn probe_ranges(
    core: Core,
    preset: &Preset,
    mut msgs: Vec<Msg>,
    passes: u32,
) -> Vec<(u32, u32)> {
    msgs.sort_by_key(|m| m.pass);
    let mut probe = PassJob::from_preset("probe", core, preset, msgs, Engine::Interp);
    let (mut next_msg, mut visited) = (0, BTreeSet::new());
    for p in 0..passes {
        let _ = probe.before_pass(p, &mut next_msg);
        visited.extend(probe_pass(&mut probe.core, &probe.cfg).2);
    }
    pc_ranges(visited)
}

/// The ranges to arm with for `mode`, as `arm_for` takes them: `None` for [`RangeMode::Extent`]
/// (`arm_for` scans the core itself and `boot` is not called), the probed ranges of `passes` passes
/// for [`RangeMode::Probe`]. `boot` makes the fresh boot of the core the probe runs on.
pub fn ranges_for(
    mode: RangeMode,
    boot: impl FnOnce() -> Result<Core, String>,
    preset: &Preset,
    msgs: &[Msg],
    passes: u32,
) -> Result<Option<Vec<(u32, u32)>>, String> {
    match mode {
        RangeMode::Extent => Ok(None),
        RangeMode::Probe => Ok(Some(probe_ranges(boot()?, preset, msgs.to_vec(), passes))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::live_presets::boot_core;
    use crate::runner::periph_run::PeriphOpts;
    use crate::runner::uop_pass::code_ranges;
    use crate::runner::uop_report::measure::{synthetic_image, synthetic_preset, SYNTH_CODE};

    const CODE: u32 = SYNTH_CODE;

    #[test]
    fn pc_ranges_merge_ascending_pcs_and_end_a_range_at_a_gap() {
        assert_eq!(pc_ranges([]), Vec::new());
        assert_eq!(pc_ranges([CODE]), vec![(CODE, CODE + 1 + RANGE_TAIL)]);
        let r = pc_ranges([
            CODE,
            CODE + 1,
            CODE + 2,
            CODE + 3 + RANGE_GAP,
            CODE + 4 + RANGE_GAP,
        ]);
        assert_eq!(
            r,
            vec![
                (CODE, CODE + 3 + RANGE_TAIL),
                (CODE + 3 + RANGE_GAP, CODE + 5 + RANGE_GAP + RANGE_TAIL),
            ],
            "RANGE_GAP unlisted parcels between two pcs split them"
        );
        assert_eq!(
            pc_ranges([CODE, CODE + RANGE_GAP]),
            vec![(CODE, CODE + RANGE_GAP + 1 + RANGE_TAIL)],
            "one fewer joins them"
        );
    }

    #[test]
    fn pc_ranges_leave_out_pcs_the_engine_would_refuse() {
        let below = CODE_WINDOW.start - 1;
        assert_eq!(
            pc_ranges([0, below, 0x12_5c88, CODE_WINDOW.end, 0x22_0000]),
            vec![(0x12_5c88, 0x12_5c88 + 1 + RANGE_TAIL)]
        );
        assert_eq!(pc_ranges([0, 5, CODE_WINDOW.end + 7]), Vec::new());
        // the last pc of the window is kept and its range is clamped to the window
        assert_eq!(
            pc_ranges([CODE_WINDOW.end - 1]),
            vec![(CODE_WINDOW.end - 1, CODE_WINDOW.end)]
        );
    }

    #[test]
    fn range_modes_are_named_on_the_command_line() {
        assert_eq!(RangeMode::named("extent"), Some(RangeMode::Extent));
        assert_eq!(RangeMode::named("probe"), Some(RangeMode::Probe));
        assert_eq!(RangeMode::named("Probe"), None);
        assert_eq!(RangeMode::named(""), None);
    }

    /// The synthetic core, plus a non-zero data parcel `far` short words past the program start.
    fn boot(far: Option<u32>) -> (Core, Preset) {
        let preset = synthetic_preset();
        let mut image = synthetic_image();
        if let Some(far) = far {
            let at = (CODE + far) as usize * 2;
            image.resize(at + 2, 0);
            image[at..].copy_from_slice(&0x7777u16.to_le_bytes());
        }
        let core = boot_core(&image, &preset, &PeriphOpts::default()).unwrap();
        (core, preset)
    }

    #[test]
    fn the_probe_covers_exactly_the_pcs_the_passes_run() {
        let (core, preset) = boot(None);
        // the poll jump and the ten NOPs run: pcs CODE..=CODE+10, and the jump has 3 parcels
        assert_eq!(
            probe_ranges(core, &preset, vec![], 3),
            vec![(CODE, CODE + 10 + 1 + RANGE_TAIL)]
        );
    }

    #[test]
    fn the_mode_decides_whether_a_probe_boots_and_runs() {
        let (_, preset) = boot(None);
        let no_boot = || -> Result<Core, String> { panic!("the extent mode does not probe") };
        assert_eq!(
            ranges_for(RangeMode::Extent, no_boot, &preset, &[], 5),
            Ok(None)
        );
        let boots = || Ok(boot(None).0);
        assert_eq!(
            ranges_for(RangeMode::Probe, boots, &preset, &[], 5),
            Ok(Some(vec![(CODE, CODE + 10 + 1 + RANGE_TAIL)]))
        );
        let fails = || Err("no image".to_string());
        assert_eq!(
            ranges_for(RangeMode::Probe, fails, &preset, &[], 5),
            Err("no image".into())
        );
    }

    #[test]
    fn no_passes_probe_no_pcs() {
        let (core, preset) = boot(None);
        assert_eq!(probe_ranges(core, &preset, vec![], 0), Vec::new());
    }

    #[test]
    fn data_that_is_never_executed_is_in_the_extent_ranges_but_not_in_the_probe_ranges() {
        let far = 2 * RANGE_GAP;
        let (core, preset) = boot(Some(far));
        assert_eq!(
            code_ranges(&core),
            vec![
                // the poll jump ends in a zero parcel: the last non-zero parcel is CODE + 11
                (CODE, CODE + 11 + 1 + RANGE_TAIL),
                (CODE + far, CODE + far + 1 + RANGE_TAIL),
            ],
            "the extent scan takes the data parcel for code"
        );
        let probe = probe_ranges(core, &preset, vec![], 2);
        assert_eq!(probe, vec![(CODE, CODE + 10 + 1 + RANGE_TAIL)]);
        // and the engine accepts what the probe found
        let (mut core, _) = boot(Some(far));
        let cfg = crate::core::UopConfig::default();
        assert!(core.enable_uop(cfg, &probe).is_ok());
    }
}
