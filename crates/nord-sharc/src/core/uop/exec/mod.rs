//! Block execution: the hot loop that runs a block's uops through the handler table, and the
//! checked loop used near events.
//!
//! OWNER: W2-DRIVE (this file, `exit`, `event`, `h_core`); the other files of this directory
//! belong to their families (`h_float` W2-FLOAT, `h_fixed` W2-FIXED, `h_move` and `mem` W2-MOVE,
//! `h_flow` W2-FLOW, `pe` W1-KERN).
//!
//! Dispatch protocol (design 11.1, one implementation here, mirrored by the native tier): a uop
//! that returns `Ok` adds `u.steps` to `done`; if `u.flags & UF_CB` the loop calls
//! `core.latch_cb_overflow()` when `cb_wrap != 0`; if `u.flags & UF_FIN` it calls
//! `core.finish_step(u.pc, kind, next)` (kind and next from the decode-cache entry at `u.pc`) and
//! treats a result other than `next` as `Jump`. `Jump`: `done` includes the terminator's steps and
//! the pc is `ctx.next_pc`. `Smc`: `done` includes the store's steps and the pc is the next uop's
//! `pc` (or `Block::end_pc` after the last). `Fault`, `Guard`, `Slow`: the uop did not run, the pc
//! is `u.pc`.
//!
//! Skeleton: both loops run nothing and report `Exit::Slow` at the entry uop with no step done,
//! which the driver treats as "the caller runs `Core::step`" (design section 8, the always-correct
//! fallback). Spec: `docs/specs/sharc-uop.md`; exit protocol: design section 4.5.
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

/// Runs block `id` from uop `from_idx` with `room` steps before the event deadline, with no
/// per-uop room check (the caller guarantees `room` covers the rest of the block). Returns the
/// exit, the index of the uop it happened at and the steps accounted before it.
///
/// # Safety
/// One raw pointer is derived from `core` for the handlers; `hot` must be the table the block
/// was built for and `id`, `from_idx` must be in range. Handlers never re-enter this function.
pub(crate) unsafe fn run_block(
    core: &mut Core,
    hot: &UopHot,
    id: u32,
    from_idx: u32,
    room: u32,
) -> (Exit, u32, u32) {
    let _ = (core, hot, id, room);
    (Exit::Slow, from_idx, 0)
}

/// [`run_block`] with a room check before every uop: the loop used when the event deadline falls
/// inside the block. It stops with `Exit::Slow` at the first uop that does not fit.
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
    let _ = (core, hot, id, room);
    (Exit::Slow, from_idx, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    #[test]
    fn the_skeleton_loops_run_nothing_and_ask_for_the_interpreter() {
        let hot = UopHot::new();
        let mut c = core_with(&[0x0001, 0x0001]);
        let before = (c.pc, c.steps);
        let r = unsafe { run_block(&mut c, &hot, 0, 3, 8) };
        assert_eq!(r, (Exit::Slow, 3, 0));
        let r = unsafe { run_block_checked(&mut c, &hot, 0, 5, 1) };
        assert_eq!(r, (Exit::Slow, 5, 0));
        assert_eq!(
            (c.pc, c.steps),
            before,
            "a Slow exit with no step leaves the core alone"
        );
    }
}
