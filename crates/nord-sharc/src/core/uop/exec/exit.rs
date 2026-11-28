//! Exit materialization and exit counters.
//!
//! OWNER: W2-DRIVE. Rule 2 of the exit protocol (design 4.5): at a uop boundary the core is
//! materialized as `pc = u.pc` and `steps = base_steps + done`; the dispatch loop adds `u.steps`
//! to `done` after each `Ok` uop. The skeleton implements exactly that, the package adds the
//! per-kind handling (fault replay, link following, SMC revalidation).
use super::super::config::UopStats;
use super::super::types::{Exit, UopCtx};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

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
}
