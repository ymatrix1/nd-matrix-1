//! Handlers of the flow range `0x400..0x500`.
//!
//! OWNER: W2-FLOW. One file per concern (design P11): `pack` (operand layout), `branch` (jump,
//! call, return), `loops` (`DO`), `misc` (`IDLE`, system-register load); `enc` (encodings),
//! `harness` (stand-in dispatch loop), `rig` (two cores side by side), `diff` (one instruction on
//! random states), `diff_chain` (loops, calls, delayed branches), `diff_events` (`IDLE`,
//! interrupts, random programs) and `allocs` (allocation counts) are test-only.
//!
//! | Kind | Instruction | Exits |
//! |---|---|---|
//! | `JUMP` | `IF cond JUMP/CALL target`, immediate or PC-relative, delayed or not | `Ok` (not taken, delayed branch issued, or the target is the next pc), `Jump` |
//! | `JUMP_I` | `IF cond JUMP/CALL (Mm, Ii)`, delayed or not | as `JUMP` |
//! | `RET` | `IF cond RTS/RTI`, delayed or not | as `JUMP`, `Fault` (empty call stack, nothing changed) |
//! | `DO_COUNT` | `DO ... UNTIL LCE` with an immediate or register count | `Ok`, `Jump` (loop back-edge when the body is one instruction), `Fault` (count 0, nothing changed) |
//! | `DO_UNTIL` | `DO ... UNTIL cond` (arithmetic loop) | `Ok`, `Jump` |
//! | `IDLE` | `IDLE` | `Ok`, `Jump`, `Slow` (a peripheral model is present, nothing changed) |
//! | `SYS_LOAD` | `sysreg = immediate` | `Ok` |
//!
//! The protocol, all of it design 4.5, P1, P2 and P3:
//!
//! * A terminator (everything above but `SYS_LOAD`) runs the tail of `exec_cached` itself:
//!   `pc = if loops.is_empty() && pending.is_none() { next } else { finish_step(u.pc, kind, next) }`
//!   with `next` the branch destination. That is the flow family contract the translator relies
//!   on (`BlockBuilder::note_terminator`): the loop back-edge of a branch that is also the last
//!   instruction of a loop body and the countdown of a delayed branch happen inside the handler,
//!   so the translator never puts `UF_FIN` on a terminator (it would run the step twice). A
//!   delayed branch returns `Ok` with `pending` set and its two delay slots follow in the same
//!   block; their uops carry `UF_FIN`, which is where the branch resolves.
//! * The result pc decides the exit: equal to the fall-through pc (`u.pc + len`, the block's
//!   `end_pc` for a last uop) is `Ok`, anything else writes `ctx.next_pc` and returns
//!   `Exit::Jump`.
//! * No handler writes `ctx.done`, `core.pc` or `core.steps`: the dispatch loop adds `u.steps` to
//!   `done` for `Ok`, `Jump` and `Smc` and materializes at every exit (`exit::materialize`). A
//!   handler that needs the interpreter's view (timers, the interrupt controller and the system
//!   registers read `steps` and `pc`) materializes first, exactly as `INTERP` does.
//! * Handlers that read a system register through `Core::read_ureg` materialize first: `SYS_LOAD`
//!   (the write), `RTI` and `DO_COUNT` with a register count (`FADDR`, `DADDR`, `PC`, `EMUCLK`,
//!   `EMUCLK2` and `TCOUNT` read `pc` or `steps`; `diff_chain.rs` covers each of them).
//! * A handler that returns `Fault` or `Slow` has changed nothing; the driver replays the
//!   instruction through `Core::step`, which raises the real fault.
//! * Allocation (design 9.3, P14): a steady-state call of every handler allocates nothing except
//!   `DO_UNTIL`. KNOWN LIMITATION, inherited from v1: it runs `Core::exec_do_until`, whose
//!   `refresh_check` (`arith_loop.rs`, not a W2-FLOW file) collects the addresses of the loop
//!   body in a `Vec`, so a `DO_UNTIL` handler allocates once per capacity doubling of that walk
//!   (measured by `allocs.rs`: 1 for a body of 1 instruction, 2 for 5, 5 for 38), as the
//!   interpreter and the `Interp` fallback do. It happens once per entry of an arithmetic loop,
//!   not per pass. G2's "no allocation in a steady pass" does not hold for a pass that enters
//!   such a loop until `refresh_check` walks the body twice (count, then index) instead of
//!   collecting it; `allocs.rs` pins the handler to exactly the allocations of that walk, so
//!   nothing else can creep in.
//! * A taken branch that pokes the interrupt controller (`RTI`, a call onto a nearly full PC
//!   stack, a loop stack overflow) is a block end, so the next block entry sees `irq.due == 0`
//!   (`Steps` row of the guard) and the interpreter's prologue runs before anything else. The
//!   one place a block continues after a branch is a delayed one: it is lowered only when its
//!   handler cannot poke (`RTI` with delay slots and the `(LA)` and `(CI)` jump modifiers stay
//!   `Interp`).
use super::super::types::{Exit, Handler, Uop, UopCtx, UopKind};
use crate::core::Core;
use crate::visa::Kind;

/// Wraps a safe handler body in the `Handler` ABI (design 4.4): the body gets the core, the
/// context and the uop as references.
macro_rules! entry {
    ($(#[$m:meta])* $name:ident => $body:path) => {
        $(#[$m])*
        pub(super) unsafe extern "C" fn $name(
            ctx: *mut $crate::core::uop::types::UopCtx,
            u: *const $crate::core::uop::types::Uop,
        ) -> u32 {
            // SAFETY: the dispatch loop calls a handler with the live context of the block and a
            // uop of that block; `ctx.core` is the one pointer to the core (design 4.4) and no
            // other reference to the core or the context is alive during the call.
            let (ctx, u) = unsafe { (&mut *ctx, &*u) };
            let core = unsafe { &mut *ctx.core };
            $body(core, ctx, u) as u32
        }
    };
}

pub mod branch;
pub mod loops;
pub mod misc;
pub mod pack;

#[cfg(test)]
pub(in crate::core) mod enc;
#[cfg(test)]
pub(in crate::core) mod harness;
#[cfg(test)]
pub(in crate::core) mod rig;

#[cfg(test)]
mod allocs;
#[cfg(test)]
mod diff;
#[cfg(test)]
mod diff_chain;
#[cfg(test)]
mod diff_events;

/// `IF cond JUMP/CALL target`: `a` = target, `b` = branch word.
pub const JUMP: UopKind = UopKind(UopKind::FLOW_BASE);
/// `IF cond JUMP/CALL (Mm, Ii)`: `a` = [`pack::indirect`], `b` = branch word.
pub const JUMP_I: UopKind = UopKind(UopKind::FLOW_BASE + 1);
/// `IF cond RTS/RTI`: `b` = branch word.
pub const RET: UopKind = UopKind(UopKind::FLOW_BASE + 2);
/// `DO ... UNTIL LCE`: `a` = last body pc, `b` = count.
pub const DO_COUNT: UopKind = UopKind(UopKind::FLOW_BASE + 3);
/// `DO ... UNTIL cond`: `a` and `b` = the 48-bit instruction word.
pub const DO_UNTIL: UopKind = UopKind(UopKind::FLOW_BASE + 4);
/// `IDLE`: `b` = branch word (length).
pub const IDLE: UopKind = UopKind(UopKind::FLOW_BASE + 5);
/// `sysreg = immediate`: `a` = register code, `b` = value. Not a terminator.
pub const SYS_LOAD: UopKind = UopKind(UopKind::FLOW_BASE + 6);

/// Every kind of the family, in table order.
pub const KINDS: [UopKind; 7] = [JUMP, JUMP_I, RET, DO_COUNT, DO_UNTIL, IDLE, SYS_LOAD];

/// The instruction kind `finish_step` is called with by every flow handler but `DO_COUNT` and
/// `DO_UNTIL`. VERIFIED: `branch.rs::finish_step` only compares the kind with `Kind::T12a` (the
/// `DO ... UNTIL LCE` instruction itself must not take the back-edge of the loop it just pushed),
/// checked by `diff.rs` over every flow encoding of the census.
pub(super) const NOT_DO: Kind = Kind::T8a;

/// The `exec_cached` tail of a flow instruction at `u.pc` whose destination is `dest` and whose
/// fall-through pc is `next`: runs `finish_step` when a loop or a delayed branch is active, then
/// picks the exit from the resulting pc.
#[inline(always)]
pub(super) fn end_step(
    core: &mut Core,
    ctx: &mut UopCtx,
    u: &Uop,
    kind: Kind,
    dest: u32,
    next: u32,
) -> Exit {
    let pc = if core.loops.is_empty() && core.pending.is_none() {
        dest
    } else {
        core.finish_step(u.pc, kind, dest)
    };
    if pc == next {
        Exit::Ok
    } else {
        ctx.next_pc = pc;
        Exit::Jump
    }
}

/// Fills the flow range of `t`.
pub(crate) fn register(t: &mut [Handler]) {
    let table: [(UopKind, Handler); 7] = [
        (JUMP, branch::h_jump),
        (JUMP_I, branch::h_jump_i),
        (RET, branch::h_ret),
        (DO_COUNT, loops::h_do_count),
        (DO_UNTIL, loops::h_do_until),
        (IDLE, misc::h_idle),
        (SYS_LOAD, misc::h_sys_load),
    ];
    for (kind, h) in table {
        t[usize::from(kind.0)] = h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::{bail_handler, KindFamily};

    /// Distinct body so the linker cannot fold it with another handler.
    unsafe extern "C" fn marker_handler(_: *mut UopCtx, _: *const Uop) -> u32 {
        0xFEED
    }

    #[test]
    fn the_kinds_are_distinct_and_inside_the_flow_range() {
        let range = KindFamily::Flow.range();
        let mut seen = Vec::new();
        for k in KINDS {
            assert!(range.contains(&usize::from(k.0)), "{k:?}");
            assert_eq!(k.family(), Some(KindFamily::Flow));
            assert!(!seen.contains(&k.0), "{k:?} twice");
            seen.push(k.0);
        }
    }

    /// Compares handler pointers only against a copy of the same value (function addresses of
    /// separately named items are not guaranteed to be identical across codegen units).
    #[test]
    fn registration_fills_exactly_the_kinds_of_the_family_and_nothing_else() {
        let marker: Handler = marker_handler;
        let mut t = vec![marker; usize::from(UopKind::MAX)];
        register(&mut t);
        for (i, h) in t.iter().enumerate() {
            let untouched = *h as usize == marker as usize;
            let ours = KINDS.iter().any(|k| usize::from(k.0) == i);
            assert_eq!(untouched, !ours, "slot {i:#x}");
        }
        let bail: Handler = bail_handler;
        let distinct = KINDS
            .iter()
            .filter(|k| t[usize::from(k.0)] as usize != bail as usize)
            .count();
        assert_eq!(distinct, KINDS.len(), "no kind keeps the bail handler");
    }

    #[test]
    fn the_handler_table_of_the_engine_carries_the_flow_handlers() {
        let table = crate::core::uop::hot::UopHot::handler_table();
        let bail: Handler = bail_handler;
        for k in KINDS {
            assert_ne!(table[usize::from(k.0)] as usize, bail as usize, "{k:?}");
        }
    }
}
