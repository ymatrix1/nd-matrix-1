//! Handlers of the core range `0x000..0x100`: `NOP`, `INTERP`, `FIN`, `BAIL`.
//!
//! OWNER: W2-DRIVE. `INTERP` looks the instruction up at `u.pc` exactly as `Core::step` does
//! (`decode_at`: `isa_at`, `decode_cache.get`, then `fetch_and_cache`) and runs `exec_cached`;
//! `FIN` runs `finish_step`; `NOP` only accounts its steps (the dispatch loop adds `u.steps`).
//!
//! The `INTERP` rules (design 11.1 P3, review of 2026-11-28; each one has a test below):
//! 1. It materializes first (`core.pc = u.pc`, `core.steps = base_steps + done`), because
//!    `exec_cached` and the system-register paths read both (timers count `steps`).
//! 2. After `exec_cached` the core is already at the next boundary. If `core.pc != u.b` (a flow
//!    instruction, a loop back-edge or a resolved delayed branch moved it) the handler returns
//!    `Exit::Jump` with `ctx.next_pc = core.pc`; a block never continues past such a uop.
//! 3. It then checks `uop_run.flags & RF_SMC` (an interpreted store may hit translated code) and
//!    returns `Exit::Smc`, and re-checks the deadline: `ctx.room` is lowered to the steps left
//!    before `irq.due`, so the dispatch loop runs the rest of the block through its room check and
//!    the first uop that no longer fits exits `Slow` in front of it (design 4.5 rule 4; the
//!    handler cannot return `Slow` itself, because `Slow` means the uop did not run).
//! 4. It adds 1 to `uop_run.stats.steps_interp_uop` (gate G2 reads it; the driver counts only
//!    `steps_uop`). An `Err(Fault)` from `exec_cached` is stored in `uop_run.fault` and returned
//!    as `Exit::Fault` with the partial effects the interpreter left: no replay, `pc` and `steps`
//!    are those of the faulting instruction, exactly like `Core::step`.
use super::super::types::{bail_handler, Exit, Handler, Uop, UopCtx, UopKind};
use super::event;
use crate::core::uop::lower::decode::decode_at;
use crate::core::uop::run::RF_SMC;
use crate::core::Core;

/// Fills the core range of `t`.
pub(crate) fn register(t: &mut [Handler]) {
    t[usize::from(UopKind::NOP.0)] = nop_handler;
    t[usize::from(UopKind::INTERP.0)] = interp_handler;
    t[usize::from(UopKind::FIN.0)] = fin_handler;
    t[usize::from(UopKind::BAIL.0)] = bail_handler;
}

/// `finish_step` of the instruction at `pc`, with kind and next pc from the decode cache like
/// `exec_cached` takes them. Returns `(next, resolved)`: `resolved != next` means a loop back-edge
/// or a delayed branch moved the pc. Shared by the `FIN` handler and the dispatch loop (`UF_FIN`).
#[inline]
pub(crate) fn finish_at(core: &mut Core, pc: u32) -> (u32, u32) {
    let c = decode_at(core, pc);
    (c.next_pc, core.finish_step(pc, c.insn.kind, c.next_pc))
}

/// `NOP`: nothing to do, the dispatch loop accounts the steps.
unsafe extern "C" fn nop_handler(_ctx: *mut UopCtx, _u: *const Uop) -> u32 {
    Exit::Ok as u32
}

/// `INTERP`: one instruction through `exec_cached` (rules 1 to 4 above).
///
/// # Safety
/// `ctx` points to the live context of the running block, `ctx.core` to its core and `u` to a uop
/// of that block; the dispatch loop upholds all three.
unsafe extern "C" fn interp_handler(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let (ctx, u) = (&mut *ctx, &*u);
    let core = &mut *ctx.core;
    core.pc = u.pc;
    core.steps = ctx.base_steps + u64::from(ctx.done);
    let c = decode_at(core, u.pc);
    if let Err(f) = core.exec_cached(&c) {
        core.uop_run.fault = Some(f);
        return Exit::Fault as u32;
    }
    core.uop_run.stats.steps_interp_uop += 1;
    ctx.room = ctx.room.min(event::due_room(core, ctx.base_steps));
    if core.pc != u.b {
        ctx.next_pc = core.pc;
        return Exit::Jump as u32;
    }
    if core.uop_run.flags & RF_SMC != 0 {
        return Exit::Smc as u32;
    }
    Exit::Ok as u32
}

/// `FIN`: runs `finish_step` for the instruction at `u.pc`; `Jump` when it moved the pc.
///
/// # Safety
/// As [`interp_handler`].
unsafe extern "C" fn fin_handler(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let (ctx, u) = (&mut *ctx, &*u);
    let (next, to) = finish_at(&mut *ctx.core, u.pc);
    if to == next {
        return Exit::Ok as u32;
    }
    ctx.next_pc = to;
    Exit::Jump as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::lower::builder::BlockBuilder;
    use crate::core::uop::types::{KindFamily, NONE};

    /// Distinct body so the linker cannot fold it with `bail_handler`.
    unsafe extern "C" fn marker_handler(_: *mut UopCtx, _: *const Uop) -> u32 {
        0xFEED
    }

    /// Compares handler pointers only against a copy of the same value (function addresses of
    /// separately named items are not guaranteed to be identical across codegen units).
    #[test]
    fn registration_writes_only_the_core_range() {
        let marker: Handler = marker_handler;
        let mut t = vec![marker; usize::from(UopKind::MAX)];
        register(&mut t);
        let ours = [UopKind::NOP, UopKind::INTERP, UopKind::FIN, UopKind::BAIL].map(|k| k.0);
        for (i, h) in t.iter().enumerate() {
            let untouched = *h as usize == marker as usize;
            let expected = ours.contains(&(i as u16));
            assert_eq!(untouched, !expected, "slot {i:#x}");
            assert!(!expected || KindFamily::Core.range().contains(&i));
        }
        assert_eq!(
            unsafe { t[usize::from(UopKind::BAIL.0)](std::ptr::null_mut(), std::ptr::null()) },
            Exit::Slow as u32
        );
    }

    /// Runs the handler of `kind` once on a context for `core` at `base` with `done` steps behind.
    fn call(
        core: &mut Core,
        kind: UopKind,
        u: &Uop,
        base: u64,
        done: u32,
        room: u32,
    ) -> (u32, UopCtx) {
        let mut ctx = UopCtx::new(core as *mut Core, base, room);
        ctx.done = done;
        let mut t = vec![bail_handler as Handler; usize::from(UopKind::MAX)];
        register(&mut t);
        let r = unsafe { t[usize::from(kind.0)](&mut ctx, u) };
        (r, ctx)
    }

    /// The `INTERP` uop of the instruction at `pc` of `core`.
    fn interp_uop(core: &mut Core, pc: u32) -> Uop {
        let mut b = BlockBuilder::new(pc, false, false);
        b.interp(&decode_at(core, pc));
        b.uops()[0]
    }

    #[test]
    fn interp_materializes_first_runs_the_instruction_and_counts_it() {
        // rule 1 and 4: timers and the like read `steps`, so the handler writes pc and steps
        // from the context before the interpreter body runs
        let mut c = core_with(&[0x0001, 0x0001, 0x0001]);
        c.irq.due = u64::MAX;
        let u = interp_uop(&mut c, CODE + 1);
        (c.pc, c.steps) = (0xDEAD, 0xBEEF); // stale: the block never writes them between uops
        let (r, ctx) = call(&mut c, UopKind::INTERP, &u, 100, 2, 50);
        assert_eq!(Exit::from_u32(r), Some(Exit::Ok));
        assert_eq!(
            (c.pc, c.steps),
            (CODE + 2, 103),
            "base 100 + 2 done + this step"
        );
        assert_eq!(c.uop_run.stats.steps_interp_uop, 1);
        assert_eq!(ctx.room, 50, "a deadline far away does not lower the room");
    }

    #[test]
    fn interp_signals_jump_when_the_pc_left_the_next_instruction() {
        // rule 2: JUMP 0x120010 is a flow instruction
        let mut c = core_with(&[0x063E, 0x0012, 0x0010]);
        c.irq.due = u64::MAX;
        let u = interp_uop(&mut c, CODE);
        let (r, ctx) = call(&mut c, UopKind::INTERP, &u, 0, 0, 50);
        assert_eq!(Exit::from_u32(r), Some(Exit::Jump));
        assert_eq!((ctx.next_pc, c.pc), (CODE + 0x10, CODE + 0x10));
        // a plain instruction does not touch next_pc
        let mut n = core_with(&[0x0001]);
        n.irq.due = u64::MAX;
        let u = interp_uop(&mut n, CODE);
        let (r, ctx) = call(&mut n, UopKind::INTERP, &u, 0, 0, 5);
        assert_eq!((Exit::from_u32(r), ctx.next_pc), (Some(Exit::Ok), NONE));
    }

    #[test]
    fn interp_reports_a_store_into_translated_code_and_lowers_the_room_after_a_poke() {
        // rule 3: `DM(0x90000) = R0` writes the words that hold the first parcels of the code
        let mut c = core_with(&[0x1100, 0x0009, 0x0000, 0x0001]);
        c.irq.due = u64::MAX;
        c.enable_uop(Default::default(), &[(CODE, CODE + 4)])
            .unwrap();
        let u = interp_uop(&mut c, CODE);
        let (r, _) = call(&mut c, UopKind::INTERP, &u, 0, 0, 10);
        assert_eq!(Exit::from_u32(r), Some(Exit::Smc));
        assert_eq!(c.uop_run.flags & RF_SMC, RF_SMC);
        // IRPTL = 0 pokes the controller: the deadline is now, the room is 0
        let mut p = core_with(&[0x0FFA, 0x0000, 0x0001]);
        p.irq.due = u64::MAX;
        let u = interp_uop(&mut p, CODE);
        let (r, ctx) = call(&mut p, UopKind::INTERP, &u, 7, 0, 10);
        assert_eq!(
            (Exit::from_u32(r), p.irq.due, ctx.room),
            (Some(Exit::Ok), 0, 0)
        );
    }

    #[test]
    fn interp_stores_the_fault_and_leaves_the_state_of_the_faulting_instruction() {
        // rule 4: the interpreter does not model this encoding; `Core::step` leaves the state
        let mut c = core_with(&[0xd70a, 0x3c23, 0xd70a]);
        c.irq.due = u64::MAX;
        let u = interp_uop(&mut c, CODE);
        let want = c.step().unwrap_err();
        let (r, _) = call(&mut c, UopKind::INTERP, &u, 4, 3, 10);
        assert_eq!(Exit::from_u32(r), Some(Exit::Fault));
        assert_eq!(c.uop_run.fault, Some(want));
        assert_eq!(
            (c.pc, c.steps),
            (CODE, 7),
            "pc of the fault, steps up to it"
        );
        assert_eq!(
            c.uop_run.stats.steps_interp_uop, 0,
            "a faulted step is not a step"
        );
    }

    #[test]
    fn nop_accounts_nothing_itself_and_fin_resolves_a_loop_back_edge() {
        let mut c = core_with(&[0x0001]);
        let u = Uop::new(UopKind::NOP, 1, CODE, 0, CODE + 1);
        let before = (c.pc, c.steps);
        let (r, _) = call(&mut c, UopKind::NOP, &u, 0, 0, 1);
        assert_eq!(
            (Exit::from_u32(r), (c.pc, c.steps)),
            (Some(Exit::Ok), before)
        );
        // FIN on a loop end: the frame says the loop body ends at CODE and has 2 passes left
        let mut l = core_with(&[0x0001, 0x0001]);
        l.loops.push(crate::core::types::LoopFrame {
            start: CODE,
            end: CODE,
            remaining: 2,
            ..Default::default()
        });
        let fin = Uop::new(UopKind::FIN, 0, CODE, 0, CODE + 1);
        let (r, ctx) = call(&mut l, UopKind::FIN, &fin, 0, 0, 1);
        assert_eq!((Exit::from_u32(r), ctx.next_pc), (Some(Exit::Jump), CODE));
        assert_eq!(l.loops.last().map(|f| f.remaining), Some(1));
        // the last pass falls through: the frame is popped and the pc stays the next instruction
        let (r, _) = call(&mut l, UopKind::FIN, &fin, 0, 0, 1);
        assert_eq!(Exit::from_u32(r), Some(Exit::Ok));
        assert!(l.loops.is_empty());
    }
}
