//! Event split: how many steps a block may run before the interrupt controller must look at the
//! core again.
//!
//! OWNER: W2-DRIVE. `Core::step` begins with `if self.steps >= self.irq.due && self.sequencer_event()`
//! (`exec.rs`), so the prologue of the step that starts at clock `s` does nothing while `s < due`:
//! a block entered at `steps` may run `due - steps` steps without changing what the interpreter
//! would do. `poke_irq` sets `due` to 0, which makes every later room 0 until a prologue ran.
//! VERIFIED: the rule by `event_split_matches_interpreter` (a scheduled interrupt lands on the
//! same step under the engine and under `Core::run`, with a block that straddles it), the poke by
//! `poke_forces_checked_loop` (a `poke_irq` inside a block makes the rest run through the
//! interpreter prologue), both in this file.
use crate::core::Core;

/// Steps left before `min(limit, irq.due)`, clamped to `u32`.
#[inline]
pub(crate) fn room(core: &Core, limit: u64) -> u32 {
    let end = limit.min(core.irq.due);
    u32::try_from(end.saturating_sub(core.steps)).unwrap_or(u32::MAX)
}

/// Steps from the clock `base_steps` up to the controller deadline, clamped to `u32`. A handler
/// whose uop may have moved the deadline (an interpreted instruction, a wrap latch, a peripheral
/// access) lowers `UopCtx::room` with it, which makes the dispatch loop re-check every following
/// uop against the room (design 4.5 rule 4).
#[inline]
pub(crate) fn due_room(core: &Core, base_steps: u64) -> u32 {
    u32::try_from(core.irq.due.saturating_sub(base_steps)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::interrupts::{IRPTEN, VECTOR_PARCELS};
    use crate::core::support::{core_with, load, CODE};
    use crate::core::uop::drive::fixture::{arm_one_interp_block, run_both, Setup};
    use crate::core::uop::types::{Exit, GuardFail};
    use crate::core::uop::verify::compare::exact_diff;

    #[test]
    fn room_is_the_distance_to_the_nearer_of_the_limit_and_the_deadline() {
        let mut c = core_with(&[0x0001]);
        c.steps = 10;
        c.irq.due = u64::MAX;
        assert_eq!(room(&c, 30), 20);
        c.irq.due = 14;
        assert_eq!(room(&c, 30), 4);
        c.irq.due = 10;
        assert_eq!(room(&c, 30), 0, "an event due now leaves no room");
        c.irq.due = 0;
        assert_eq!(room(&c, 30), 0, "poke_irq (due = 0) leaves no room");
        c.irq.due = u64::MAX;
        assert_eq!(room(&c, 5), 0, "a limit already passed");
        assert_eq!(room(&c, u64::MAX), u32::MAX, "clamped to u32");
    }

    #[test]
    fn due_room_counts_from_the_block_entry_and_never_underflows() {
        let mut c = core_with(&[0x0001]);
        c.irq.due = 100;
        assert_eq!(due_room(&c, 40), 60);
        assert_eq!(due_room(&c, 100), 0);
        assert_eq!(due_room(&c, 250), 0, "a deadline already passed");
        c.irq.due = u64::MAX;
        assert_eq!(due_room(&c, 0), u32::MAX);
    }

    /// Entry of the service routine behind vector 11.
    const ISR: u32 = 0x13_0000 + 0x100;

    /// Interrupts enabled, vector 11 unmasked and jumping to an `RTI` at [`ISR`].
    fn vector_11(c: &mut Core) {
        c.set_ivt_base(0x13_0000);
        // word 0 of the vector is `JUMP ISR`: parcels least significant first
        load(
            c,
            0x13_0000 + VECTOR_PARCELS * 11,
            &[ISR as u16, (ISR >> 16) as u16, 0x063e],
        );
        load(c, ISR, &[0x0bbe]);
        c.mode1 |= IRPTEN;
        c.imask |= 1 << 11;
    }

    /// 120 NOPs (three blocks of at most 48 steps) with interrupt 11 scheduled at clock `cycle`.
    fn irq_setup(cycle: u64) -> Setup {
        Setup::nops(120).with(move |c| {
            vector_11(c);
            c.schedule_interrupt(cycle, 11);
        })
    }

    #[test]
    fn event_split_matches_interpreter() {
        // the deadline lands on every kind of clock: before the first step, right after it, inside
        // a block, on a block end and inside the next block
        for cycle in [
            0, 1, 2, 3, 5, 11, 12, 13, 17, 23, 24, 25, 39, 47, 48, 49, 60, 95, 96, 97,
        ] {
            let r = run_both(&irq_setup(cycle), 110);
            assert_eq!(r.diff, None, "cycle {cycle}: {r:?}");
            assert_eq!(r.taken, 1, "cycle {cycle}: the interrupt was taken once");
            // step 0 runs through `Core::step` (the schedule poked the deadline to 0); blocks run
            // from step 1, and what the engine does at the deadline is what the comparison with the
            // interpreter above checks: a block that ran one step too far moves the interrupt
            assert!(
                r.stats.steps_uop >= cycle.saturating_sub(1),
                "cycle {cycle}: {r:?}"
            );
            if cycle >= 2 {
                assert!(r.exercised(), "cycle {cycle}: blocks ran: {r:?}");
            }
        }
    }

    #[test]
    fn a_deadline_inside_the_block_runs_the_checked_loop_and_one_core_step() {
        let setup = irq_setup(20);
        let (mut want, mut got) = (setup.core(), setup.armed());
        // a first Core::step computes `due`: the schedule left it at 0
        assert_eq!((want.run(1), got.run_uop(1)), (None, None));
        assert_eq!((want.irq.due, got.irq.due), (20, 20));
        // no limit: the deadline alone cuts the block, in front of the 20th step
        assert!(got.uop_try(u64::MAX));
        assert_eq!(want.run(19), None);
        assert_eq!(exact_diff(&want, &got), None);
        let s = got.uop_stats();
        assert_eq!(
            (s.steps_uop, s.exits[Exit::Slow as usize]),
            (19, 1),
            "{s:?}"
        );
        assert_eq!(got.steps, 20);
        // the deadline is reached: no block starts, the next step takes the interrupt
        let steps_rows = got.uop_stats().guard_fail[GuardFail::Steps as usize];
        assert!(!got.uop_try(u64::MAX));
        assert_eq!(
            got.uop_stats().guard_fail[GuardFail::Steps as usize],
            steps_rows + 1,
            "no room left"
        );
        assert_eq!((want.step(), got.step()), (Ok(()), Ok(())));
        assert_eq!(exact_diff(&want, &got), None);
        assert_eq!(got.interrupts_taken(), 1);
    }

    #[test]
    fn a_poked_core_starts_no_block_until_a_prologue_looked_at_the_controller() {
        let setup = Setup::nops(12);
        let (mut want, mut got) = (setup.core(), setup.armed());
        want.irq.due = 0;
        got.irq.due = 0;
        assert!(!got.uop_try(100), "room is 0 after a poke");
        assert_eq!(got.uop_stats().guard_fail[GuardFail::Steps as usize], 1);
        assert_eq!((want.run(12), got.run_uop(12)), (None, None));
        assert_eq!(exact_diff(&want, &got), None);
        assert!(got.irq.due > 0, "the interpreter prologue recomputed it");
    }

    #[test]
    fn poke_forces_checked_loop() {
        // `IRPTL = 0x0800` (two parcels) latches interrupt 11 and pokes the controller from inside
        // a block. The interpreter takes the interrupt before the step that follows the write; so
        // must the engine, although the block was entered with room for all 48 steps.
        let mut program = vec![0x0001, 0x0001, 0x0ffa, 0x0800];
        program.resize(60, 0x0001);
        let setup = Setup::program(&program).with(vector_11);
        let mut stepped = setup.core();
        assert_eq!(stepped.run(3), None);
        assert_eq!(
            stepped.irq.irptl >> 11 & 1,
            1,
            "the program latches vector 11 at its third step"
        );
        assert_eq!(stepped.irq.due, 0, "and pokes the controller");
        // one block over the first 20 instructions, the register write in the middle of it (the
        // translator would cut after the write; a family's block must still be right without)
        let (mut want, mut got) = (setup.core(), setup.core());
        arm_one_interp_block(&mut got, (CODE, CODE + 21));
        assert!(got.uop_try(50), "the block starts with room for all of it");
        assert_eq!(want.run(3), None);
        assert_eq!(exact_diff(&want, &got), None);
        let s = got.uop_stats();
        assert_eq!(
            (s.steps_uop, s.exits[Exit::Slow as usize]),
            (3, 1),
            "NOP, NOP, the write, then Slow in front of the step after it: {s:?}"
        );
        assert_eq!(got.irq.due, 0);
        // no block starts until a prologue looked at the controller: the next step takes the
        // interrupt, as the interpreter's does
        assert!(!got.uop_try(50));
        assert_eq!(got.uop_stats().guard_fail[GuardFail::Steps as usize], 1);
        assert_eq!((want.step(), got.step()), (Ok(()), Ok(())));
        assert_eq!(exact_diff(&want, &got), None);
        assert_eq!(got.interrupts_taken(), 1);
        // and the rest of the program agrees
        assert_eq!((want.run(40), got.run_uop(40)), (None, None));
        assert_eq!(exact_diff(&want, &got), None);
    }
}
