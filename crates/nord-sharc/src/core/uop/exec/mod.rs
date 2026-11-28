//! Block execution: the loop that runs a block's uops through the handler table, hot or with a
//! room check before every uop.
//!
//! OWNER: W2-DRIVE (this file, `dispatch`, `exit`, `event`, `h_core`); the other files of this
//! directory belong to their families (`h_float` W2-FLOAT, `h_fixed` W2-FIXED, `h_move` and `mem`
//! W2-MOVE, `h_flow` W2-FLOW, `pe` W1-KERN).
//!
//! Dispatch protocol (design 11.1 P2, one implementation in `dispatch.rs`, mirrored by the native
//! tier). A handler returns an [`Exit`](super::types::Exit):
//!
//! | Exit | The uop | The loop |
//! |---|---|---|
//! | `Ok` | ran | adds `u.steps` to `done`; `UF_FIN` calls `finish_step(u.pc, kind, next)` with kind and next from the decode-cache entry at `u.pc` and treats a result other than `next` as `Jump`; `UF_CB` calls `latch_cb_overflow()` when `cb_wrap != 0` (in that order, like `exec_cached`); then the next uop |
//! | `Jump` | ran, wrote `ctx.next_pc` | `done` includes its steps; the pc is `ctx.next_pc` |
//! | `Smc` | ran (a store hit a code page) | `done` includes its steps; the pc is the next uop's `pc`, `Block::end_pc` after the last |
//! | `Fault`, `Guard`, `Slow` | did NOT run | `done` does not include it; the pc is `u.pc` |
//!
//! A handler that may poke the interrupt controller (`INTERP`, system registers, memory outside
//! the SRAM windows, the wrap latch) lowers `ctx.room` to the steps left before `irq.due`
//! (`event::due_room`). The loop re-checks `room` against the rest of the block after every uop,
//! and from then on runs in the checked mode: the first uop with `done + steps > room` exits
//! `Slow` in front of itself, so the interpreter's `Core::step` prologue sees the event at exactly
//! the step where it would have seen it. This replaces the design text of rule 4 ("the uop ends
//! with `return Slow`"): `Slow` means the uop did not run, so a uop that already ran cannot
//! return it. Rule 2 holds on every exit: `pc` and `steps` are materialized at the uop boundary
//! (`exit::materialize`).
//!
//! Spec: `docs/specs/sharc-uop.md`; exit protocol: design section 4.5.
pub mod dispatch;
pub mod event;
pub mod exit;
pub mod h_core;
pub mod h_fixed;
pub mod h_float;
pub mod h_flow;
pub mod h_move;
pub mod mem;
pub mod pe;

use super::hot::UopHot;
use super::types::Exit;
use crate::core::Core;

/// Runs block `id` from uop `from_idx` with `room` steps before the event deadline. The loop starts
/// without a per-uop room check when `room` covers the rest of the block (the common case) and
/// switches to the checked mode by itself as soon as a handler lowers the room. Returns the exit,
/// the index of the uop it happened at (the uop count after the last uop for an `Ok` at the end of
/// the block) and the steps accounted before it.
///
/// An out-of-range `id` or `from_idx` answers `(Slow, from_idx, 0)` with the core untouched.
///
/// # Safety
/// `hot` must be the table the block was built for. One raw pointer is derived from `core` for the
/// handlers; handlers never re-enter this function.
pub(crate) unsafe fn run_block(
    core: &mut Core,
    hot: &UopHot,
    id: u32,
    from_idx: u32,
    room: u32,
) -> (Exit, u32, u32) {
    dispatch::run(core, hot, id, from_idx, room, false)
}

/// [`run_block`] with a room check before every uop from the start: the loop used when the event
/// deadline falls inside the block. It stops with `Exit::Slow` in front of the first uop that does
/// not fit.
///
/// # Safety
/// As [`run_block`].
pub(crate) unsafe fn run_block_checked(
    core: &mut Core,
    hot: &UopHot,
    id: u32,
    from_idx: u32,
    room: u32,
) -> (Exit, u32, u32) {
    dispatch::run(core, hot, id, from_idx, room, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::CODE;
    use crate::core::uop::drive::fixture::{interp_block, nop_core};

    #[test]
    fn the_two_entries_differ_only_in_where_the_room_check_starts() {
        let (hot, mut hot_core) = interp_block(6);
        let mut checked_core = nop_core(6);
        // room for the whole block: both run it through
        let a = unsafe { run_block(&mut hot_core, &hot, 0, 0, 100) };
        let b = unsafe { run_block_checked(&mut checked_core, &hot, 0, 0, 100) };
        assert_eq!(a, (Exit::Ok, 6, 6));
        assert_eq!(a, b);
        assert_eq!(hot_core.state_mismatch(&checked_core), None);
        assert_eq!((hot_core.pc, hot_core.steps), (CODE + 6, 6));
        // room for four of six: the hot entry notices by itself (its caller's promise was wrong
        // or a handler lowered the room), both stop in front of the fifth uop
        let (mut x, mut y) = (nop_core(6), nop_core(6));
        let a = unsafe { run_block(&mut x, &hot, 0, 0, 4) };
        let b = unsafe { run_block_checked(&mut y, &hot, 0, 0, 4) };
        assert_eq!(a, (Exit::Slow, 4, 4));
        assert_eq!(a, b);
        assert_eq!((x.pc, x.steps), (CODE + 4, 4));
        assert_eq!(x.state_mismatch(&y), None);
    }

    #[test]
    fn a_block_or_entry_that_does_not_exist_asks_for_the_interpreter_and_changes_nothing() {
        let (hot, mut c) = interp_block(3);
        let before = (c.pc, c.steps);
        for (id, from) in [(1, 0), (0, 3), (0, 99)] {
            let r = unsafe { run_block(&mut c, &hot, id, from, 10) };
            assert_eq!(r, (Exit::Slow, from, 0), "block {id} from {from}");
            let r = unsafe { run_block_checked(&mut c, &hot, id, from, 10) };
            assert_eq!(r, (Exit::Slow, from, 0));
        }
        assert_eq!((c.pc, c.steps), before);
    }
}
