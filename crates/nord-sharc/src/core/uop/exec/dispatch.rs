//! The dispatch loop: one function for the hot and the checked block runs.
//!
//! OWNER: W2-DRIVE. The loop is the one implementation of the dispatch protocol P2 (the header of
//! `exec/mod.rs` lists it); the native tier mirrors it. All access to the core and to the context
//! goes through the two raw pointers derived at entry (aliasing discipline, design 14 risk 10):
//! no `&mut Core` outlives a call, handlers never re-enter the loop, nothing here panics.
use super::super::hot::UopHot;
use super::super::types::{bail_handler, Block, Exit, Handler, Uop, UopCtx, UF_CB, UF_FIN};
use super::{event, exit, h_core};
use crate::core::Core;

/// Runs block `id` from uop `from_idx`. `checked` starts the loop in the room-checked mode; the
/// loop also switches to it as soon as a handler lowers `ctx.room` below what the rest of the
/// block needs (a deadline moved by an interpreted instruction, a wrap latch or `finish_step`).
/// In the checked mode the first uop with `done + steps > room` exits `Slow` in front of itself.
///
/// # Safety
/// `hot` must be the table the block was built for. Out-of-range `id` or `from_idx` answer
/// `Slow` with no step done; a handler pointer is only called with a live context and a uop of
/// the block.
pub(super) unsafe fn run(
    core: &mut Core,
    hot: &UopHot,
    id: u32,
    from_idx: u32,
    room: u32,
    checked: bool,
) -> (Exit, u32, u32) {
    let Some(block) = hot.blocks.get(id as usize) else {
        return (Exit::Slow, from_idx, 0);
    };
    let span = block.first as usize..block.first as usize + block.len as usize;
    let from = from_idx as usize;
    let Some(uops) = hot.arena.get(span).filter(|u| from < u.len()) else {
        return (Exit::Slow, from_idx, 0);
    };
    let cp: *mut Core = core;
    let mut ctx = UopCtx::new(cp, (*cp).steps, room);
    let ctx: *mut UopCtx = &mut ctx;
    // steps from the entry uop to the end of the block: the room the hot loop needs
    let need = steps_from(block, from_idx);
    let mut checked = checked || room < need;
    let mut i = from;
    while i < uops.len() {
        let u = &uops[i];
        let s = u32::from(u.steps);
        if checked && (*ctx).done + s > (*ctx).room {
            return leave(cp, ctx, Exit::Slow, i, u.pc);
        }
        let handler = hot
            .handlers
            .get(usize::from(u.h))
            .copied()
            .unwrap_or(bail_handler as Handler);
        let mut e = Exit::from_u32(handler(ctx, u)).unwrap_or(Exit::Slow);
        if matches!(e, Exit::Fault | Exit::Guard | Exit::Slow) {
            return leave(cp, ctx, e, i, u.pc);
        }
        (*ctx).done += s;
        if e != Exit::Jump && u.flags & (UF_FIN | UF_CB) != 0 && after(cp, ctx, u) {
            e = Exit::Jump;
        }
        match e {
            Exit::Ok => {}
            Exit::Jump => return leave(cp, ctx, e, i, (*ctx).next_pc),
            _ => return leave(cp, ctx, e, i, exit::resume_pc(uops, i, block.end_pc)),
        }
        checked |= (*ctx).room < need;
        i += 1;
    }
    exit::materialize(&mut *cp, &*ctx, block.end_pc);
    (Exit::Ok, uops.len() as u32, (*ctx).done)
}

/// Steps from uop `from` to the end of `block`: the room the hot loop needs.
#[inline]
pub(crate) fn steps_from(block: &Block, from: u32) -> u32 {
    let behind = block.cum.get(from as usize).map_or(0, |c| u32::from(*c));
    block.steps.saturating_sub(behind)
}

/// Materializes the core for the exit `e` at uop `i` with continuation pc `pc`.
///
/// # Safety
/// `cp` and `ctx` are the live pointers of [`run`].
#[inline(always)]
unsafe fn leave(cp: *mut Core, ctx: *mut UopCtx, e: Exit, i: usize, pc: u32) -> (Exit, u32, u32) {
    (*ctx).exit_idx = i as u32;
    exit::materialize(&mut *cp, &*ctx, pc);
    (e, i as u32, (*ctx).done)
}

/// What follows a uop that completed: `finish_step` for `UF_FIN` (true when it moved the pc, the
/// target is then in `ctx.next_pc`), then the wrap latch for `UF_CB`, in the order of
/// `exec_cached`. Both may poke the controller, so the room follows the deadline afterwards.
///
/// # Safety
/// `cp` and `ctx` are the live pointers of [`run`].
#[cold]
unsafe fn after(cp: *mut Core, ctx: *mut UopCtx, u: &Uop) -> bool {
    let mut jumped = false;
    if u.flags & UF_FIN != 0 {
        let (next, to) = h_core::finish_at(&mut *cp, u.pc);
        if to != next {
            (*ctx).next_pc = to;
            jumped = true;
        }
    }
    if u.flags & UF_CB != 0 && (*cp).cb_wrap.get() != 0 {
        (*cp).latch_cb_overflow();
    }
    (*ctx).room = (*ctx).room.min(event::due_room(&*cp, (*ctx).base_steps));
    jumped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::lower::builder::BlockBuilder;
    use crate::core::uop::types::{GuardClass, UopKind};

    thread_local! {
        /// Uops the recording handler ran, by `u.a`.
        static RAN: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Handler of the test kinds: records `u.a`, and answers with the exit that `u.b` names.
    unsafe extern "C" fn probe(ctx: *mut UopCtx, u: *const Uop) -> u32 {
        let (ctx, u) = (&mut *ctx, &*u);
        RAN.with(|r| r.borrow_mut().push(u.a));
        match u.b {
            1 => ctx.next_pc = 0x12_0777,
            9 => (*ctx.core).irq.due = 0, // a poke that does not lower the room itself
            10 => ctx.room = 0,           // a handler that lowers the room, like INTERP
            _ => {}
        }
        if u.b == 9 || u.b == 10 {
            return Exit::Ok as u32;
        }
        u.b.min(5)
    }

    const KIND: UopKind = UopKind(UopKind::MOVE_BASE + 1);

    /// A hot table whose test kind is `probe`, and one block of `n` one-step uops at `CODE`;
    /// uop `k` carries `a = k` and answers with `exits[k]` (default `Ok`).
    fn block(n: u32, exits: &[(u32, u32)], flags: &[(u32, u8)]) -> (UopHot, Core) {
        let mut hot = UopHot::new();
        hot.handlers[usize::from(KIND.0)] = probe;
        let mut b = BlockBuilder::new(CODE, false, false);
        for k in 0..n {
            let b_arg = exits.iter().find(|(i, _)| *i == k).map_or(0, |e| e.1);
            let mut u = Uop::new(KIND, 1, CODE + k, k, b_arg);
            u.flags = flags.iter().find(|(i, _)| *i == k).map_or(0, |f| f.1);
            b.push(u);
            b.set_end(CODE + k + 1);
        }
        let (block, uops) = b.finish(GuardClass::Plain);
        hot.install(vec![block], uops);
        let mut core = core_with(&vec![0x0001; n as usize + 2]);
        core.steps = 1000;
        core.irq.due = u64::MAX;
        (hot, core)
    }

    fn ran() -> Vec<u32> {
        RAN.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }

    #[test]
    fn a_block_that_runs_through_materializes_the_end_and_all_its_steps() {
        let (hot, mut c) = block(4, &[], &[]);
        let r = unsafe { run(&mut c, &hot, 0, 0, 100, false) };
        assert_eq!(r, (Exit::Ok, 4, 4));
        assert_eq!((c.pc, c.steps), (CODE + 4, 1004));
        assert_eq!(ran(), [0, 1, 2, 3]);
    }

    #[test]
    fn each_exit_materializes_where_the_protocol_says() {
        // Jump: the terminator's step is done, the pc is ctx.next_pc
        let (hot, mut c) = block(4, &[(1, 1)], &[]);
        assert_eq!(
            unsafe { run(&mut c, &hot, 0, 0, 100, false) },
            (Exit::Jump, 1, 2)
        );
        assert_eq!((c.pc, c.steps), (0x12_0777, 1002));
        // Smc: the store's step is done, the pc is the next uop's
        let (hot, mut c) = block(4, &[(1, 4)], &[]);
        assert_eq!(
            unsafe { run(&mut c, &hot, 0, 0, 100, false) },
            (Exit::Smc, 1, 2)
        );
        assert_eq!((c.pc, c.steps), (CODE + 2, 1002));
        // ... and the end of the block after the last uop
        let (hot, mut c) = block(2, &[(1, 4)], &[]);
        assert_eq!(
            unsafe { run(&mut c, &hot, 0, 0, 100, false) },
            (Exit::Smc, 1, 2)
        );
        assert_eq!(c.pc, CODE + 2);
        // Fault, Guard, Slow: the uop did not run, the pc is u.pc and its step is not counted
        for (code, exit) in [(2, Exit::Fault), (3, Exit::Guard), (5, Exit::Slow)] {
            let (hot, mut c) = block(4, &[(2, code)], &[]);
            assert_eq!(unsafe { run(&mut c, &hot, 0, 0, 100, false) }, (exit, 2, 2));
            assert_eq!((c.pc, c.steps), (CODE + 2, 1002), "{exit:?}");
        }
        ran();
    }

    #[test]
    fn mid_block_entry_starts_at_the_uop_and_counts_from_there() {
        let (hot, mut c) = block(5, &[], &[]);
        c.steps = 50;
        ran();
        let r = unsafe { run(&mut c, &hot, 0, 3, 100, false) };
        assert_eq!(r, (Exit::Ok, 5, 2), "two uops left");
        assert_eq!((c.pc, c.steps), (CODE + 5, 52));
        assert_eq!(ran(), [3, 4]);
    }

    #[test]
    fn the_checked_loop_stops_in_front_of_the_first_uop_that_does_not_fit() {
        for room in 0..=5 {
            let (hot, mut c) = block(4, &[], &[]);
            let r = unsafe { run(&mut c, &hot, 0, 0, room, true) };
            let want = if room >= 4 {
                (Exit::Ok, 4, 4)
            } else {
                (Exit::Slow, room, room)
            };
            assert_eq!(r, want, "room {room}");
            assert_eq!(c.steps, 1000 + u64::from(want.2));
            assert_eq!(ran().len() as u32, want.2, "room {room}");
        }
        // the hot entry with too little room is the checked loop (the caller's guarantee failed)
        let (hot, mut c) = block(4, &[], &[]);
        assert_eq!(
            unsafe { run(&mut c, &hot, 0, 0, 2, false) },
            (Exit::Slow, 2, 2)
        );
        ran();
    }

    #[test]
    fn a_handler_that_lowers_the_room_makes_the_rest_of_the_block_run_checked() {
        // uop 1 sets ctx.room = 0: uop 2 must not run, exactly like a deadline now
        let (hot, mut c) = block(4, &[(1, 10)], &[]);
        let r = unsafe { run(&mut c, &hot, 0, 0, 100, false) };
        assert_eq!(r, (Exit::Slow, 2, 2));
        assert_eq!((c.pc, c.steps), (CODE + 2, 1002));
        assert_eq!(ran(), [0, 1]);
    }

    #[test]
    fn fin_and_cb_flags_run_finish_step_and_the_wrap_latch_after_the_uop() {
        // uop 1 ends a loop body (loop end = its pc, 2 passes left): the pc goes back to start
        let (hot, mut c) = block(4, &[], &[(1, UF_FIN)]);
        c.loops.push(crate::core::types::LoopFrame {
            start: CODE,
            end: CODE + 1,
            remaining: 2,
            ..Default::default()
        });
        let r = unsafe { run(&mut c, &hot, 0, 0, 100, false) };
        assert_eq!(r, (Exit::Jump, 1, 2));
        assert_eq!((c.pc, c.steps, c.loops[0].remaining), (CODE, 1002, 1));
        // the same uop with a single pass left falls through and pops the frame
        let (hot, mut c) = block(3, &[], &[(1, UF_FIN)]);
        c.loops.push(crate::core::types::LoopFrame {
            start: CODE,
            end: CODE + 1,
            remaining: 1,
            ..Default::default()
        });
        assert_eq!(
            unsafe { run(&mut c, &hot, 0, 0, 100, false) },
            (Exit::Ok, 3, 3)
        );
        assert!(c.loops.is_empty());
        // UF_CB: the wrap latch clears the flag, sets the sticky bit and pokes the deadline, so
        // the next uop no longer fits
        let (hot, mut c) = block(3, &[], &[(0, UF_CB)]);
        c.cb_wrap.set(1);
        let r = unsafe { run(&mut c, &hot, 0, 0, 100, false) };
        assert_eq!(r, (Exit::Slow, 1, 1), "due = 0 after the latch");
        assert_eq!((c.cb_wrap.get(), c.irq.due), (0, 0));
        // no wrap, no latch
        let (hot, mut c) = block(3, &[], &[(0, UF_CB)]);
        assert_eq!(
            unsafe { run(&mut c, &hot, 0, 0, 100, false) },
            (Exit::Ok, 3, 3)
        );
        ran();
    }

    #[test]
    fn a_poke_the_handler_does_not_report_is_not_seen_until_the_next_entry() {
        // pins the protocol: only `ctx.room` makes a running block re-check the deadline, so a
        // handler that can poke must lower it (INTERP does); the block itself runs to its end
        let (hot, mut c) = block(3, &[(0, 9)], &[]);
        assert_eq!(
            unsafe { run(&mut c, &hot, 0, 0, 100, false) },
            (Exit::Ok, 3, 3)
        );
        assert_eq!(c.irq.due, 0);
        ran();
    }

    #[test]
    fn bad_ids_and_unknown_handlers_ask_for_the_interpreter_without_running_anything() {
        let (hot, mut c) = block(2, &[], &[]);
        for (id, from) in [(9, 0), (0, 2), (0, 7)] {
            let r = unsafe { run(&mut c, &hot, id, from, 100, false) };
            assert_eq!(r, (Exit::Slow, from, 0), "block {id} from {from}");
        }
        // a handler outside the table and one that answers a code that is no exit
        let mut hot = hot;
        hot.arena[0].h = UopKind::MAX + 3;
        assert_eq!(
            unsafe { run(&mut c, &hot, 0, 0, 100, false) },
            (Exit::Slow, 0, 0)
        );
        hot.arena[0].h = KIND.0;
        hot.arena[0].b = 77; // probe answers min(77, 5) = Slow
        assert_eq!(
            unsafe { run(&mut c, &hot, 0, 0, 100, false) },
            (Exit::Slow, 0, 0)
        );
        assert_eq!((c.pc, c.steps), (CODE, 1000));
        ran();
    }
}
