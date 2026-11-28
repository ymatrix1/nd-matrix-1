//! Shadow checking: run an engine on a state, rewind, run the interpreter for the same number of
//! steps and compare (design 6.6). The interpreter result wins, so a wrong engine result heals.
//!
//! OWNER: W3-FUZZ. Off by default and never in the audio path: it copies the memory images and
//! allocates a second core. A core with peripheral models cannot be replayed (a device read or
//! write has side effects the rewind does not undo), so the engine runs unchecked there and the
//! result is [`Shadow::Skipped`] (counted as `shadow_skipped` by the caller). Skeleton: complete
//! as a function over any engine closure; wiring `UopConfig::shadow_every` into `uop_try` and the
//! SRAM undo log that replaces the image copy are W3-FUZZ.
use super::compare::{exact_diff, ExactDiff};
use super::snap::CoreSnap;
use crate::core::Core;

/// Result of one shadow check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shadow {
    /// The engine and the interpreter agree.
    Match,
    /// They differ; the core now holds the interpreter's state. `a` of the diff is the
    /// interpreter, `b` the engine.
    Mismatch(ExactDiff),
    /// The core has peripherals: the engine ran, nothing was compared.
    Skipped,
}

/// Runs `engine` once on `core` and checks it against the interpreter from the same start state:
/// the reference is `Core::run` for the steps the engine took.
pub fn shadow_run(core: &mut Core, engine: impl FnOnce(&mut Core)) -> Shadow {
    shadow_run_with(core, engine, |c, done| {
        let _ = c.run(done);
    })
}

/// [`shadow_run`] with a caller-supplied reference run: `reference(core, steps_the_engine_took)`
/// replays the same work on the interpreter (a pass loop that also reads the mailbox, say).
pub fn shadow_run_with(
    core: &mut Core,
    engine: impl FnOnce(&mut Core),
    reference: impl FnOnce(&mut Core, u64),
) -> Shadow {
    let start = CoreSnap::capture(core);
    let (sram, iop) = (core.sram.clone(), core.iop.clone());
    let steps0 = core.steps;
    engine(core);
    if core.periph.is_some() {
        return Shadow::Skipped;
    }
    let done = core.steps - steps0;
    let mut got = Core::new(&[]);
    CoreSnap::capture(core).restore(&mut got);
    got.sram.copy_from_slice(&core.sram);
    got.iop.copy_from_slice(&core.iop);
    start.restore(core);
    core.sram.copy_from_slice(&sram);
    core.iop.copy_from_slice(&iop);
    reference(core, done);
    exact_diff(core, &got).map_or(Shadow::Match, Shadow::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::gen::{core_with_code, fill_state, random_program, Rng};

    #[test]
    fn a_correct_engine_matches() {
        let mut c = core_with_code(&random_program(&mut Rng::new(2), 10));
        fill_state(&mut c, &mut Rng::new(4), false);
        assert_eq!(
            shadow_run(&mut c, |c| {
                let _ = c.run_uop(6);
            }),
            Shadow::Match
        );
        assert_eq!(c.steps, 6);
    }

    #[test]
    fn a_custom_reference_replays_the_same_work() {
        let mut c = core_with_code(&[0x0001; 8]);
        let mut calls = 0;
        let s = shadow_run_with(
            &mut c,
            |c| {
                let _ = c.run(4);
            },
            |c, done| {
                calls += 1;
                assert_eq!(done, 4);
                let _ = c.run(done);
            },
        );
        assert_eq!((s, calls), (Shadow::Match, 1));
    }

    #[test]
    fn a_wrong_engine_is_reported_and_the_interpreter_result_is_kept() {
        let mut c = core_with_code(&[0x0001; 8]);
        let mut want = core_with_code(&[0x0001; 8]);
        want.run(5);
        let s = shadow_run(&mut c, |c| {
            c.run(5);
            c.r[0] ^= 1; // the "bug"
        });
        let Shadow::Mismatch(d) = s else {
            panic!("not detected: {s:?}")
        };
        assert_eq!(d.field, "r");
        assert!(d.a.starts_with("[0,") && d.b.starts_with("[1,"), "{d}");
        assert_eq!(
            exact_diff(&c, &want),
            None,
            "self-healed to the interpreter state"
        );
    }

    #[test]
    fn a_memory_write_by_the_engine_is_detected_and_undone() {
        let mut c = core_with_code(&[0x0001; 4]);
        let s = shadow_run(&mut c, |c| {
            c.run(2);
            c.sram[0x100] = 0x55;
        });
        assert!(
            matches!(s, Shadow::Mismatch(ref d) if d.field == "sram"),
            "{s:?}"
        );
        assert_eq!(c.sram[0x100], 0);
    }
}
