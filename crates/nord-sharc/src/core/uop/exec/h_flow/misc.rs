//! `IDLE` and the system-register load.
//!
//! OWNER: W2-FLOW. `SYS_LOAD` reads the interpreter's view of the machine (the timers count
//! `steps`, register groups 6 and 7 read `pc`), so it materializes first, like `INTERP` (design
//! 4.5 rule 5). `IDLE` only latches the sleep flag and pokes the deadline, which read neither, so
//! it does not.
//!
//! `SYS_LOAD` is the design's `Sys`: a write of an immediate into a system register. It is a block
//! end (the register may flip PEyen, the interrupt masks or the timers, i.e. the state the entry
//! guard of the next block assumed: `cut.rs::touches_sys`), so the "deadline re-check" of design
//! 7.3 is the next block entry: a write that pokes the interrupt controller leaves `irq.due == 0`,
//! the `Steps` row of the guard sees no room and the interpreter's prologue runs next, before any
//! uop. Nothing follows `SYS_LOAD` inside its block (a system load in the first delay slot is
//! never translated, `group.rs`), so there is no uop left to re-check.
use super::end_step;
use super::pack::Branch;
use crate::core::uop::exec::exit::materialize;
use crate::core::uop::types::{Exit, Uop, UopCtx};
use crate::core::Core;
use crate::visa::Kind;

/// `IDLE`. With a peripheral model present `enter_idle` lets the host DMA finish
/// (`periph_boot_wait`), which can write translated code and raise interrupts through paths a
/// block must not run through, so the uop asks for the interpreter before changing anything.
pub(super) fn idle(core: &mut Core, ctx: &mut UopCtx, u: &Uop) -> Exit {
    if core.periph.is_some() {
        return Exit::Slow;
    }
    core.enter_idle();
    let next = u.pc.wrapping_add(Branch(u.b).len());
    end_step(core, ctx, u, Kind::T21c, next, next)
}

/// `ureg = immediate` for a system register: the load of `exec_move_op` (`LoadImm`), with its
/// SIMD companion write. The universal register write is the interpreter's own
/// (`Core::write_ureg`), so every system register behaves as it does there; the step is finished
/// by the dispatch loop (`UF_FIN` when the instruction is a loop end or a delay slot), not here.
pub(super) fn sys_load(core: &mut Core, ctx: &mut UopCtx, u: &Uop) -> Exit {
    let (ureg, value) = (u.a as u8, u.b);
    materialize(core, ctx, u.pc);
    core.write_ureg(ureg, value);
    // VERIFIED: the SIMD companions of `moves.rs::exec_move_op(LoadImm)`; the first arm is dead
    // for system registers (codes >= 0x60) and kept so the function stays the same function,
    // `sys_load_equals_the_interpreter_for_every_system_register_code` runs all 128 codes
    if core.simd() && (ureg < 16 || ureg >> 4 == 5) {
        core.write_ureg(if ureg < 16 { 0x50 | ureg } else { ureg & 15 }, value);
    } else if core.simd() && (ureg == 0x76 || ureg == 0x78) {
        core.write_ureg(ureg + 1, value);
    }
    Exit::Ok
}

entry!(
    /// Handler of [`super::IDLE`].
    h_idle => idle
);
entry!(
    /// Handler of [`super::SYS_LOAD`].
    h_sys_load => sys_load
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::exec::h_flow::{IDLE, SYS_LOAD};
    use crate::core::uop::types::UopKind;

    fn run(core: &mut Core, kind: UopKind, a: u32, b: u32) -> (Exit, UopCtx) {
        let table = crate::core::uop::hot::UopHot::handler_table();
        let u = Uop::new(kind, 1, core.pc, a, b);
        let mut ctx = UopCtx::new(core as *mut Core, core.steps, 8);
        // SAFETY: `ctx.core` points to `core`, which outlives the call and is not used meanwhile.
        let r = unsafe { table[usize::from(kind.0)](&mut ctx, &u) };
        (Exit::from_u32(r).expect("an exit code"), ctx)
    }

    #[test]
    fn idle_puts_the_core_to_sleep_and_falls_through() {
        let mut c = core_with(&[0x0081, 0x0001]);
        c.irq.due = u64::MAX;
        let b = Branch::new(31, 1, false, false, false).0;
        let (exit, _) = run(&mut c, IDLE, 0, b);
        assert_eq!((exit, c.is_idle(), c.irq.due), (Exit::Ok, true, 0));
        assert_eq!(
            c.steps, 0,
            "the dispatch loop accounts the step, not the handler"
        );
    }

    #[test]
    fn idle_with_a_peripheral_model_asks_for_the_interpreter_and_changes_nothing() {
        let mut c = core_with(&[0x0081, 0x0001]);
        c.periph = Some(Box::default());
        c.irq.due = u64::MAX;
        let b = Branch::new(31, 1, false, false, false).0;
        let (exit, _) = run(&mut c, IDLE, 0, b);
        assert_eq!(
            (exit, c.is_idle(), c.irq.due),
            (Exit::Slow, false, u64::MAX)
        );
    }

    #[test]
    fn a_system_register_load_writes_the_register_and_a_masked_interrupt_write_pokes_the_deadline()
    {
        let mut c = core_with(&[0x0001]);
        c.irq.due = u64::MAX;
        // IMASK is system register 0x7B
        let (exit, _) = run(&mut c, SYS_LOAD, 0x7B, 0x5);
        assert_eq!((exit, c.imask, c.irq.due), (Exit::Ok, 5, 0));
        // the step count the handler saw is the entry count plus the steps done
        let mut c = core_with(&[0x0001]);
        c.steps = 10;
        let mut ctx = UopCtx::new(&mut c as *mut Core, 10, 8);
        ctx.done = 3;
        let u = Uop::new(SYS_LOAD, 1, CODE + 9, 0x7B, 1);
        let table = crate::core::uop::hot::UopHot::handler_table();
        // SAFETY: as above.
        let r = unsafe { table[usize::from(SYS_LOAD.0)](&mut ctx, &u) };
        assert_eq!(r, Exit::Ok as u32);
        assert_eq!(
            (c.pc, c.steps),
            (CODE + 9, 13),
            "materialized for the timers"
        );
    }
}
