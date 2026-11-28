//! `JUMP`, `JUMP_I` and `RET`: jumps, calls and returns, conditional and delayed.
//!
//! OWNER: W2-FLOW. Each body is `exec_flow_op` of the interpreter (`branch.rs` of `core`) with the
//! operands read from the uop instead of the decode cache, and it calls the same subroutines
//! (`Core::take_branch`, `push_return` through it, `return_from_interrupt`, `finish_step`), so the
//! effects and their order are the interpreter's by construction; `diff.rs` checks that against
//! `Core::step` on random states. What is not modelled is left to `Interp` by `lower/flow`: the
//! compute field of a conditional branch, the `(LA)` and `(CI)` jump modifiers and a delayed
//! `RTI`.
use super::pack::{indirect_regs, Branch};
use super::{end_step, NOT_DO};
use crate::core::types::Pending;
use crate::core::uop::exec::exit::materialize;
use crate::core::uop::lower::cut::DELAY_SLOTS;
use crate::core::uop::types::{Exit, Uop, UopCtx};
use crate::core::Core;

/// Whether the condition of the branch holds on PEx (`Decoded::cond_ok`: the branches never look
/// at PEy). `31` is TRUE and `cond` answers it too; the compare only skips the call.
#[inline(always)]
fn holds(core: &Core, b: Branch) -> bool {
    b.cond() == 31 || core.cond(b.cond())
}

/// `IF cond JUMP/CALL target`.
pub(super) fn jump(core: &mut Core, ctx: &mut UopCtx, u: &Uop) -> Exit {
    let b = Branch(u.b);
    let next = u.pc.wrapping_add(b.len());
    let dest = if holds(core, b) {
        core.take_branch(u.a, b.call(), b.delayed(), next)
    } else {
        next
    };
    end_step(core, ctx, u, NOT_DO, dest, next)
}

/// `IF cond JUMP/CALL (Mm, Ii)`: the target is `I[reg_i] + M[reg_m]` read when the branch runs.
pub(super) fn jump_i(core: &mut Core, ctx: &mut UopCtx, u: &Uop) -> Exit {
    let b = Branch(u.b);
    let next = u.pc.wrapping_add(b.len());
    let dest = if holds(core, b) {
        let (i, m) = indirect_regs(u.a);
        let target = core.i[i].wrapping_add(core.m[m]);
        core.take_branch(target, b.call(), b.delayed(), next)
    } else {
        next
    };
    end_step(core, ctx, u, NOT_DO, dest, next)
}

/// `IF cond RTS/RTI`. An empty call stack faults in the interpreter before anything changed, so
/// the handler answers `Fault` and the replay raises it.
pub(super) fn ret(core: &mut Core, ctx: &mut UopCtx, u: &Uop) -> Exit {
    let b = Branch(u.b);
    let next = u.pc.wrapping_add(b.len());
    if !holds(core, b) {
        return end_step(core, ctx, u, NOT_DO, next, next);
    }
    let Some(target) = core.call_stack.pop() else {
        return Exit::Fault;
    };
    if b.rti() {
        materialize(core, ctx, u.pc);
        core.return_from_interrupt();
    }
    let dest = if b.delayed() {
        core.pending = Some(Pending {
            slots: DELAY_SLOTS + 1,
            target,
            call: false,
        });
        next
    } else {
        target
    };
    end_step(core, ctx, u, NOT_DO, dest, next)
}

entry!(
    /// Handler of [`super::JUMP`].
    h_jump => jump
);
entry!(
    /// Handler of [`super::JUMP_I`].
    h_jump_i => jump_i
);
entry!(
    /// Handler of [`super::RET`].
    h_ret => ret
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::exec::h_flow::pack::indirect;
    use crate::core::uop::types::UopKind;

    fn run(core: &mut Core, kind: UopKind, a: u32, b: u32) -> (Exit, UopCtx) {
        let table = crate::core::uop::hot::UopHot::handler_table();
        let pc = core.pc;
        let u = Uop::new(kind, 1, pc, a, b);
        let mut ctx = UopCtx::new(core as *mut Core, core.steps, 8);
        // SAFETY: `ctx.core` points to `core`, which outlives the call and is not used meanwhile.
        let r = unsafe { table[usize::from(kind.0)](&mut ctx, &u) };
        (Exit::from_u32(r).expect("an exit code"), ctx)
    }

    #[test]
    fn a_taken_jump_writes_the_target_and_a_jump_to_the_next_pc_falls_through() {
        let mut c = core_with(&[0x0001; 4]);
        let b = Branch::new(31, 3, false, false, false).0;
        let (exit, ctx) = run(&mut c, super::super::JUMP, CODE + 0x40, b);
        assert_eq!((exit, ctx.next_pc), (Exit::Jump, CODE + 0x40));
        let (exit, ctx) = run(&mut c, super::super::JUMP, CODE + 3, b);
        assert_eq!(exit, Exit::Ok, "the destination is the fall-through pc");
        assert_eq!(ctx.next_pc, u32::MAX, "no pc written");
    }

    #[test]
    fn a_false_condition_falls_through_without_touching_the_stacks() {
        let mut c = core_with(&[0x0001; 4]);
        c.flags.az = false;
        let b = Branch::new(0, 3, false, true, false).0; // IF EQ CALL, AZ clear
        let (exit, _) = run(&mut c, super::super::JUMP, CODE + 0x40, b);
        assert_eq!((exit, c.call_stack.len()), (Exit::Ok, 0));
        c.flags.az = true;
        let (exit, ctx) = run(&mut c, super::super::JUMP, CODE + 0x40, b);
        assert_eq!(
            (exit, ctx.next_pc, c.call_stack.clone()),
            (Exit::Jump, CODE + 0x40, vec![CODE + 3])
        );
    }

    #[test]
    fn a_delayed_branch_sets_pending_and_stays_in_the_block() {
        let mut c = core_with(&[0x0001; 4]);
        let b = Branch::new(31, 3, true, false, false).0;
        let (exit, _) = run(&mut c, super::super::JUMP, CODE + 0x40, b);
        let p = c.pending.expect("pending set");
        assert_eq!(
            (exit, p.slots, p.target, p.call),
            (Exit::Ok, 2, CODE + 0x40, false)
        );
    }

    #[test]
    fn the_indirect_target_is_i_plus_m_read_when_the_branch_runs() {
        let mut c = core_with(&[0x0001; 4]);
        (c.i[9], c.m[12]) = (0x1000, 0x24);
        let b = Branch::new(31, 2, false, false, false).0;
        let (exit, ctx) = run(&mut c, super::super::JUMP_I, indirect(9, 12), b);
        assert_eq!((exit, ctx.next_pc), (Exit::Jump, 0x1024));
    }

    #[test]
    fn a_return_with_an_empty_stack_is_a_fault_that_changed_nothing() {
        let mut c = core_with(&[0x0001; 4]);
        let b = Branch::new(31, 1, false, false, false).0;
        let (exit, _) = run(&mut c, super::super::RET, 0, b);
        assert_eq!(
            (exit, c.pending.is_none(), c.call_stack.len()),
            (Exit::Fault, true, 0)
        );
        // a false condition does not fault
        let never = Branch::new(0, 1, false, false, false).0;
        c.flags.az = false;
        let (exit, _) = run(&mut c, super::super::RET, 0, never);
        assert_eq!(exit, Exit::Ok);
    }

    #[test]
    fn a_return_pops_the_address_and_a_delayed_one_defers_it() {
        let mut c = core_with(&[0x0001; 4]);
        c.push_return_address(CODE + 0x20);
        let b = Branch::new(31, 1, false, false, false).0;
        let (exit, ctx) = run(&mut c, super::super::RET, 0, b);
        assert_eq!(
            (exit, ctx.next_pc, c.call_stack.len()),
            (Exit::Jump, CODE + 0x20, 0)
        );
        c.push_return_address(CODE + 0x20);
        let delayed = Branch::new(31, 1, true, false, false).0;
        let (exit, _) = run(&mut c, super::super::RET, 0, delayed);
        let p = c.pending.expect("pending set");
        assert_eq!(
            (exit, p.target, p.slots, c.call_stack.len()),
            (Exit::Ok, CODE + 0x20, 2, 0)
        );
    }
}
