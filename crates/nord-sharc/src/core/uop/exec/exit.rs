//! Exit materialization and exit counters.
//!
//! OWNER: W2-DRIVE. Rule 2 of the exit protocol (design 4.5): at a uop boundary the core is
//! materialized as `pc = u.pc` and `steps = base_steps + done`; the dispatch loop adds `u.steps`
//! to `done` after each uop that ran. Which pc an exit materializes:
//!
//! | Exit | pc | steps |
//! |---|---|---|
//! | `Ok` at the end of the block | `Block::end_pc` | `base + done` |
//! | `Jump` | `ctx.next_pc` | `base + done` (the terminator's steps are in `done`) |
//! | `Smc` | [`resume_pc`]: the next uop's pc, `Block::end_pc` after the last | `base + done` (the store's steps are in `done`) |
//! | `Fault`, `Guard`, `Slow` | `u.pc` (the uop did not run) | `base + done` |
use super::super::config::UopStats;
use super::super::types::{Exit, Uop, UopCtx};
use crate::core::Core;

/// Writes the state the interpreter would have at the uop boundary `pc` after `ctx.done` steps.
#[inline]
pub(crate) fn materialize(core: &mut Core, ctx: &UopCtx, pc: u32) {
    core.pc = pc;
    core.steps = ctx.base_steps + u64::from(ctx.done);
}

/// Counts one exit of kind `e`.
#[inline]
pub(crate) fn count_exit(stats: &mut UopStats, e: Exit) {
    stats.exits[e as usize] += 1;
}

/// The pc a block continues at after the uop at index `i` completed and left through `Smc`: the
/// pc of the next uop, or `end_pc` after the last one.
#[inline]
pub(crate) fn resume_pc(uops: &[Uop], i: usize, end_pc: u32) -> u32 {
    uops.get(i + 1).map_or(end_pc, |n| n.pc)
}

/// Uops that ran in a block run that entered at uop `from` and left through `e` at uop `at`:
/// the uops before `at`, and `at` itself when it ran (`Jump`, `Smc`). An `Ok` reports the uop
/// count after the last uop as `at`.
#[inline]
pub(crate) fn uops_run(e: Exit, from: u32, at: u32) -> u32 {
    let at_ran = u32::from(matches!(e, Exit::Jump | Exit::Smc));
    (at + at_ran).saturating_sub(from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::types::UopKind;

    #[test]
    fn materialize_sets_pc_and_adds_the_done_steps_to_the_entry_count() {
        let mut c = core_with(&[0x0001]);
        c.steps = 100;
        let base = c.steps;
        let mut ctx = UopCtx::new(&mut c as *mut Core, base, 50);
        ctx.done = 7;
        materialize(&mut c, &ctx, 0x12_0003);
        assert_eq!((c.pc, c.steps), (0x12_0003, 107));
    }

    #[test]
    fn exits_are_counted_by_kind() {
        let mut s = UopStats::default();
        count_exit(&mut s, Exit::Slow);
        count_exit(&mut s, Exit::Slow);
        count_exit(&mut s, Exit::Smc);
        assert_eq!(
            (
                s.exits[Exit::Slow as usize],
                s.exits[Exit::Smc as usize],
                s.exits[0]
            ),
            (2, 1, 0)
        );
    }

    #[test]
    fn the_uops_that_ran_include_the_exit_uop_only_when_it_ran() {
        assert_eq!(uops_run(Exit::Ok, 0, 7), 7, "the whole block");
        assert_eq!(uops_run(Exit::Ok, 3, 7), 4, "a mid-block entry");
        assert_eq!(uops_run(Exit::Jump, 0, 4), 5, "the terminator ran");
        assert_eq!(uops_run(Exit::Smc, 2, 4), 3, "the store ran");
        for e in [Exit::Fault, Exit::Guard, Exit::Slow] {
            assert_eq!(
                uops_run(e, 2, 4),
                2,
                "{e:?}: the uop at the exit did not run"
            );
            assert_eq!(uops_run(e, 4, 4), 0);
        }
    }

    #[test]
    fn resume_continues_at_the_next_uop_or_falls_through_after_the_last() {
        let u = |pc| Uop::new(UopKind::NOP, 1, pc, 0, 0);
        let uops = [u(10), u(11), u(14)];
        assert_eq!(resume_pc(&uops, 0, 15), 11);
        assert_eq!(resume_pc(&uops, 1, 15), 14);
        assert_eq!(resume_pc(&uops, 2, 15), 15, "the end of the block");
        assert_eq!(resume_pc(&[], 0, 7), 7);
    }
}
