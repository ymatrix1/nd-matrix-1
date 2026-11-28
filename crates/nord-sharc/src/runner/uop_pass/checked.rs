//! The checked passes: the engine runs a pass, the core is rewound to the state after the trigger,
//! the interpreter runs the same pass and the two end states are compared with `exact_diff`.
//!
//! OWNER: W1-RUN. The core is left in the interpreter's state, so a wrong engine heals. The checks
//! allocate (two memory images and a second core): tools and tests only. A core with peripheral
//! models cannot be rewound; there the pass is unchecked and the verdict is [`Shadow::Skipped`],
//! which a caller that reports "no difference" must not read as [`Shadow::Match`].
//!
//! Layering, bottom up, so a test can hand any layer a deliberately wrong engine:
//! [`shadowed_with`] (any engine closure, the verdict), [`diff_of`] (the verdict as an optional
//! difference), [`checked_with`] (the two together). The public passes are those three around the
//! real engine loop.
use super::{begin_pass, interp_body, uop_body, uop_body_with, BlockEntry, EntryCallback};
use crate::core::{shadow_run_with, Core, ExactDiff, Shadow};
use crate::runner::pass::PassConfig;

/// The checked pass around any engine closure: `engine` runs the pass after the trigger, the core
/// is rewound to that state, the interpreter runs the same pass and the two end states are
/// compared. Returns what the engine returned and the verdict.
fn shadowed_with(
    core: &mut Core,
    cfg: &PassConfig,
    engine: impl FnOnce(&mut Core) -> (u64, bool),
) -> (u64, bool, Shadow) {
    let mut result = (0, false);
    let verdict = shadow_run_with(
        core,
        |c| result = engine(c),
        |c, _| {
            let _ = interp_body(c, cfg);
        },
    );
    (result.0, result.1, verdict)
}

/// The difference a verdict carries: `None` for a match and for a pass that could not be checked
/// (the shadowed form tells the two apart).
fn diff_of(verdict: Shadow) -> Option<ExactDiff> {
    match verdict {
        Shadow::Mismatch(d) => Some(d),
        Shadow::Match | Shadow::Skipped => None,
    }
}

/// [`shadowed_with`] with the difference, if any.
fn checked_with(
    core: &mut Core,
    cfg: &PassConfig,
    engine: impl FnOnce(&mut Core) -> (u64, bool),
) -> (u64, bool, Option<ExactDiff>) {
    let (steps, idle, verdict) = shadowed_with(core, cfg, engine);
    (steps, idle, diff_of(verdict))
}

/// [`super::lean_pass_uop`] checked against the interpreter, with the verdict of the check: the
/// pass runs under the engine, the core is rewound to the state after the trigger, the interpreter
/// runs the same pass and the two end states are compared with `exact_diff`. The core is left in
/// the interpreter's state, so a wrong engine heals. Allocates; tools and tests only. A core with
/// peripheral models cannot be rewound: there the verdict is [`Shadow::Skipped`].
pub fn lean_pass_uop_shadowed(core: &mut Core, cfg: &PassConfig) -> (u64, bool, Shadow) {
    begin_pass(core, cfg);
    shadowed_with(core, cfg, |c| uop_body(c, cfg))
}

/// [`lean_pass_uop_shadowed`] that reports every block entry of the engine's run to `on_entry`
/// (the interpreter replay is not reported).
pub fn lean_pass_uop_shadowed_entries(
    core: &mut Core,
    cfg: &PassConfig,
    on_entry: &mut dyn FnMut(BlockEntry),
) -> (u64, bool, Shadow) {
    begin_pass(core, cfg);
    shadowed_with(core, cfg, |c| {
        uop_body_with(c, cfg, &mut EntryCallback::new(on_entry))
    })
}

/// [`lean_pass_uop_shadowed`] with the difference, if any: `None` for a match and for a pass that
/// could not be checked (use the shadowed form to tell the two apart).
pub fn lean_pass_uop_checked(core: &mut Core, cfg: &PassConfig) -> (u64, bool, Option<ExactDiff>) {
    begin_pass(core, cfg);
    checked_with(core, cfg, |c| uop_body(c, cfg))
}

#[cfg(test)]
mod tests {
    use super::super::support::{self, poll_cfg, poll_core, POLL_PC};
    use super::*;
    use crate::core::UopConfig;
    use crate::runner::system::lean_pass;

    /// A poll core armed for the engine, started: the state a pass body begins from.
    fn started() -> Core {
        let mut c = poll_core();
        super::super::arm_uop(&mut c, UopConfig::default(), POLL_PC).unwrap();
        begin_pass(&mut c, &poll_cfg());
        c
    }

    /// The core the interpreter leaves after one pass.
    fn interpreted() -> Core {
        let mut want = poll_core();
        lean_pass(&mut want, &poll_cfg());
        want
    }

    #[test]
    fn a_wrong_register_is_a_difference_through_the_wrapper_and_heals_to_the_interpreter() {
        let cfg = poll_cfg();
        let mut c = started();
        let (steps, idle, diff) = checked_with(&mut c, &cfg, |c| {
            let r = uop_body(c, &cfg);
            c.r[3] ^= 0x10;
            r
        });
        assert_eq!(
            (steps, idle),
            (11, true),
            "the engine's own result is reported"
        );
        let d = diff.expect("a corrupted register is a difference");
        assert_eq!(d.field, "r");
        assert_eq!(
            interpreted().state_mismatch(&c),
            None,
            "healed to the interpreter state"
        );
    }

    #[test]
    fn a_stray_store_is_found_and_undone() {
        let cfg = poll_cfg();
        let mut c = started();
        let (_, _, diff) = checked_with(&mut c, &cfg, |c| {
            let r = uop_body(c, &cfg);
            c.dm_write(0xA_0000, 0x55).unwrap();
            r
        });
        assert!(
            matches!(&diff, Some(d) if d.field == "sram"),
            "memory is compared too: {diff:?}"
        );
        assert_eq!(c.dm_read(0xA_0000), Ok(0), "and undone");
    }

    #[test]
    fn an_engine_that_steps_one_instruction_too_far_is_a_difference() {
        let cfg = poll_cfg();
        let mut c = started();
        let (_, _, diff) = checked_with(&mut c, &cfg, |c| {
            let r = uop_body(c, &cfg);
            let _ = c.step();
            r
        });
        assert!(
            diff.is_some(),
            "an extra step shows in pc, steps or the call state"
        );
    }

    /// A poll core, armed for the engine, whose engine is wrong (`support::mark_wrong_engine`:
    /// the engine equals the interpreter, so nothing could prove that a check sees a difference
    /// without a wrong one).
    fn wrong() -> Core {
        let mut c = poll_core();
        support::mark_wrong_engine(&mut c);
        super::super::arm_uop(&mut c, UopConfig::default(), POLL_PC).unwrap();
        c
    }

    /// The core the interpreter leaves after one pass of [`wrong`]'s program.
    fn interpreted_wrong() -> Core {
        let mut want = poll_core();
        support::mark_wrong_engine(&mut want);
        lean_pass(&mut want, &poll_cfg());
        want
    }

    #[test]
    fn the_public_checked_pass_reports_a_wrong_engine_as_a_difference_and_heals() {
        // the public wrapper itself, not `checked_with`: a wrapper that dropped the difference
        // (`(steps, idle, None)`) fails here
        let mut c = wrong();
        let (steps, idle, diff) = lean_pass_uop_checked(&mut c, &poll_cfg());
        assert_eq!((steps, idle), (11, true), "the engine's own result");
        let d = diff.expect("a wrong engine is a difference through the public pass");
        assert_eq!(d.field, "r", "{d}");
        assert_eq!(interpreted_wrong().state_mismatch(&c), None, "healed");
        // and every pass after it, the engine stays wrong and the check stays awake
        let (_, _, again) = lean_pass_uop_checked(&mut c, &poll_cfg());
        assert!(again.is_some_and(|d| d.field == "r"));
    }

    #[test]
    fn the_shadowed_passes_say_mismatch_for_a_wrong_engine() {
        let cfg = poll_cfg();
        let (mut a, mut b) = (wrong(), wrong());
        let (steps, idle, verdict) = lean_pass_uop_shadowed(&mut a, &cfg);
        assert_eq!((steps, idle), (11, true));
        assert!(
            matches!(&verdict, Shadow::Mismatch(d) if d.field == "r"),
            "{verdict:?}"
        );
        let mut entries = 0;
        let (steps, idle, verdict) = lean_pass_uop_shadowed_entries(&mut b, &cfg, &mut |_| {
            entries += 1;
        });
        assert_eq!((steps, idle), (11, true));
        assert!(
            matches!(&verdict, Shadow::Mismatch(d) if d.field == "r"),
            "{verdict:?}"
        );
        assert!(entries > 0, "the engine was entered: {entries}");
        assert_eq!(a.state_mismatch(&b), None, "both healed to the same state");
    }

    #[test]
    fn only_the_checked_passes_see_a_wrong_engine() {
        // the unchecked pass is as wrong as the mark says: the difference the checked forms
        // report is real, not an artefact of the replay
        let mut c = wrong();
        assert_eq!(super::super::lean_pass_uop(&mut c, &poll_cfg()), (11, true));
        assert!(interpreted_wrong().state_mismatch(&c).is_some());
    }

    #[test]
    fn the_verdict_maps_to_a_difference_only_for_a_mismatch() {
        let d = ExactDiff {
            field: "r",
            a: "1".into(),
            b: "2".into(),
        };
        assert_eq!(diff_of(Shadow::Mismatch(d.clone())), Some(d));
        assert_eq!(diff_of(Shadow::Match), None);
        assert_eq!(diff_of(Shadow::Skipped), None);
    }

    #[test]
    fn a_correct_engine_has_no_difference_and_the_shadowed_form_says_match() {
        let cfg = poll_cfg();
        let mut c = started();
        assert_eq!(
            checked_with(&mut c, &cfg, |c| uop_body(c, &cfg)),
            (11, true, None)
        );
        let mut c = poll_core();
        super::super::arm_uop(&mut c, UopConfig::default(), POLL_PC).unwrap();
        assert_eq!(
            lean_pass_uop_shadowed(&mut c, &cfg),
            (11, true, Shadow::Match)
        );
        assert_eq!(lean_pass_uop_checked(&mut c, &cfg), (11, true, None));
    }

    #[test]
    fn a_core_with_a_peripheral_model_is_skipped_not_matched() {
        let cfg = poll_cfg();
        // a core with a peripheral model cannot be rewound: the pass ran, nothing was compared
        let mut p = poll_core();
        p.enable_epdma();
        super::super::arm_uop(&mut p, UopConfig::default(), POLL_PC).unwrap();
        assert_eq!(
            lean_pass_uop_shadowed(&mut p, &cfg),
            (11, true, Shadow::Skipped)
        );
        // the checked form reports it as "no difference"
        assert_eq!(lean_pass_uop_checked(&mut p, &cfg), (11, true, None));
    }

    #[test]
    fn the_shadowed_entries_form_is_the_same_check_and_reports_the_engine_run_only() {
        let cfg = poll_cfg();
        let (mut a, mut b) = (poll_core(), poll_core());
        super::super::arm_uop(&mut a, UopConfig::default(), POLL_PC).unwrap();
        super::super::arm_uop(&mut b, UopConfig::default(), POLL_PC).unwrap();
        let want = lean_pass_uop_shadowed(&mut a, &cfg);
        let mut entries = 0u64;
        let got = lean_pass_uop_shadowed_entries(&mut b, &cfg, &mut |_| entries += 1);
        assert_eq!(got, want);
        assert_eq!(a.state_mismatch(&b), None);
        assert_eq!(a.uop_stats(), b.uop_stats(), "watching moves no counter");
        // the replay on the interpreter enters no block: only the engine's entries are reported
        assert!(entries <= b.uop_stats().blocks_run, "{entries}");
    }
}
