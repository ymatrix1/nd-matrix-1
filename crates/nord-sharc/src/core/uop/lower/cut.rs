//! Cut rules: where a basic block ends, and the instruction classes the translator needs.
//!
//! OWNER: W1-LOWER. Design 4.2: a block ends after any flow instruction (jump, call, return, DO
//! setup, stack operation, IDLE), after any load into a system register (`touches_sys`: it can
//! flip PEyen, the interrupt masks or the timers, i.e. the state the entry guard assumed), after
//! the last instruction of a statically known loop body (`translate.rs`), before `stop_pc` and at
//! `max_block_steps`. `stop_pc` (the pass idle pc) is always a block START, never an interior
//! instruction: the pass loop ends a pass when a step LANDS on it (`pc == idle_pc` after the
//! step), so the block that reaches it must end there, and `uop_try` at `pc == stop_pc` runs
//! nothing. A pc that is not known at translation time is repaired by `uop_begin_pass` (design
//! 11.1, rule P7). A cut never separates a delayed branch from its owed slots
//! (`BlockBuilder::owed_slots`); `translate.rs` asks [`cuts_after`] only when none is owed.
//!
//! Stores are NOT a cut (the v1 whole-pass blocks cut after a store that may leave the SRAM): a
//! store handler re-checks `irq.due` and the self-modified-code flag after the store (design 4.5
//! rules 4 and 6), which keeps the long straight-line stretches of DSP code in one block.
//!
//! The classes below are conservative: a wrong "yes" only splits a block or widens a guard, a
//! wrong "no" would be an exactness bug. `diff.rs` checks every "no" against `Core::step`.
use super::LowerCtx;
use crate::core::decode_cache::{CachedInsn, ComputeOp, DecodedOp, FlowOp, MoveOp};
use crate::visa::{bits, Kind};

/// Instructions that run after a delayed branch before it takes effect. VERIFIED:
/// `branch.rs::DELAY_SLOTS` is 2 (`pending.slots` starts at 3 because `finish_step` of the branch
/// itself counts one), checked by `a_delayed_branch_lands_after_exactly_two_slots` (interpreter
/// pc after one, two and three steps).
pub(in crate::core) const DELAY_SLOTS: u8 = 2;

/// First universal register code of the system registers. VERIFIED: `aot_whole.rs::may_change_guard`
/// uses the same bound for "the register file below the system registers cannot change the
/// guard", checked by `diff.rs` (sweep of every register code of every load form against
/// `Core::step`).
const SYS_UREG: u8 = 0x60;

/// What an instruction does to the flow of the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::core) enum Flow {
    /// Falls through to the next instruction.
    Plain,
    /// Ends the block (its successor is not the fall-through pc, or the dispatcher must
    /// re-enter): `slots` delay slots follow it and belong to the same block.
    End { slots: u8 },
}

fn end_with(delayed: bool) -> Flow {
    Flow::End {
        slots: if delayed { DELAY_SLOTS } else { 0 },
    }
}

/// The flow class of `c` (`aot_whole.rs::terminator`, refined for `CJUMP`: delayed only when
/// bit 34 is set, `RFRAME` is a plain instruction).
pub(in crate::core) fn flow_of(c: &CachedInsn) -> Flow {
    match c.op {
        DecodedOp::Flow(
            FlowOp::JumpDirect { delayed, .. }
            | FlowOp::JumpIndirect { delayed, .. }
            | FlowOp::Return { delayed, .. },
        ) => end_with(delayed),
        DecodedOp::Flow(FlowOp::DoUntilCounter { .. })
        | DecodedOp::DoUntil
        | DecodedOp::StackOp
        | DecodedOp::Idle => Flow::End { slots: 0 },
        DecodedOp::Type25 if is_rframe(c.w) => Flow::Plain,
        DecodedOp::Type25 => end_with(bits(c.w, 34, 34) == 1),
        _ => Flow::Plain,
    }
}

/// `RFRAME` (`branch_ext.rs::exec_type25`): `I7 = I6, I6 = DM(0, I6)`, no branch.
fn is_rframe(w: u64) -> bool {
    bits(w, 47, 41) == 0x0C && bits(w, 40, 40) == 1
}

fn sys(ureg: u8) -> bool {
    ureg >= SYS_UREG
}

/// Whether `c` may write a system register (a load into a universal register code `>= 0x60`, a
/// bit operation on one, or a form not known to be harmless): the entry guard of the following
/// instructions may no longer hold. After it the block ends. Loads from memory into a data or
/// DAG register, stores and compute cannot change guarded state.
pub(in crate::core) fn touches_sys(c: &CachedInsn) -> bool {
    let w = c.w;
    match c.op {
        DecodedOp::Move(MoveOp::LoadImm { ureg, .. }) => sys(ureg),
        DecodedOp::Move(MoveOp::RegMove { dst, .. }) => sys(dst),
        DecodedOp::Move(
            MoveOp::DirectMem { write, ureg, .. } | MoveOp::DagMem { write, ureg, .. },
        ) => !write && sys(ureg),
        DecodedOp::Move(MoveOp::RegSwap { .. }) => false,
        DecodedOp::Move(MoveOp::Other) => match c.insn.kind {
            // 4-bit data-register field: cannot name a system register
            Kind::T4a | Kind::T4b | Kind::T7a | Kind::T7b => false,
            Kind::T15a => bits(w, 40, 40) == 0 && sys(bits(w, 38, 32) as u8),
            Kind::T15b => bits(w, 40, 40) == 0 && sys(bits(w, 29, 23) as u8),
            // bit operations on system registers (T18a) and every form not listed
            _ => true,
        },
        DecodedOp::Nop
        | DecodedOp::ComputeCond { .. }
        | DecodedOp::ComputeBoth { .. }
        | DecodedOp::ModifyImm
        | DecodedOp::ShiftImm
        | DecodedOp::ShiftMove
        | DecodedOp::DualMove(_) => false,
        // flow instructions end the block anyway; the rest is not translated
        DecodedOp::Flow(_)
        | DecodedOp::StackOp
        | DecodedOp::DoUntil
        | DecodedOp::Type25
        | DecodedOp::Idle
        | DecodedOp::Unimplemented => true,
    }
}

fn compute_raises(op: &ComputeOp) -> bool {
    !matches!(
        op,
        ComputeOp::Nop | ComputeOp::Shifter { .. } | ComputeOp::Invalid
    )
}

fn opt_raises(op: &Option<ComputeOp>) -> bool {
    op.as_ref().is_some_and(compute_raises)
}

/// Whether `c` can raise an arithmetic exception (`note_arith_exception`: fixed ALU, float ALU
/// and multiplier, fixed multiplier, dual add/sub, multifunction; also inside the compute field
/// of a move or a flow instruction). A block with a fast uop for such an instruction is
/// `GuardClass::Arith`: it must not start while the exception interrupt is enabled, because the
/// handler of a fast uop would have to raise it at the right step.
pub(in crate::core) fn can_raise_arith(c: &CachedInsn) -> bool {
    match &c.op {
        DecodedOp::ComputeCond { op, .. } | DecodedOp::ComputeBoth { op } => compute_raises(op),
        DecodedOp::DualMove(dm) => opt_raises(&dm.compute),
        DecodedOp::Flow(FlowOp::JumpIndirect { compute, .. } | FlowOp::Return { compute, .. }) => {
            opt_raises(compute)
        }
        DecodedOp::Move(
            MoveOp::RegSwap { compute, .. }
            | MoveOp::RegMove { compute, .. }
            | MoveOp::DagMem { compute, .. },
        ) => opt_raises(compute),
        // the compute field of these forms is not kept in the decoded op
        DecodedOp::Move(MoveOp::Other) if matches!(c.insn.kind, Kind::T4a | Kind::T7a) => {
            compute_raises(&ComputeOp::decode(bits(c.w, 22, 0)))
        }
        _ => false,
    }
}

/// Whether `c` can be part of a block. `Unimplemented` instructions fault in the interpreter:
/// they are left to it (a barrier between blocks).
pub(in crate::core) fn translatable(c: &CachedInsn) -> bool {
    !matches!(c.op, DecodedOp::Unimplemented)
}

/// Whether the block under construction ends after `c`, which is its `steps_in_block`-th step.
/// The caller asks only when no delay slot is owed.
pub(in crate::core) fn cuts_after(cx: &LowerCtx<'_>, c: &CachedInsn, steps_in_block: u32) -> bool {
    matches!(flow_of(c), Flow::End { .. })
        || touches_sys(c)
        || steps_in_block >= cx.cfg.max_block_steps.max(1)
        || cx.stop_pc == Some(c.next_pc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::lower::decode::decode_at;
    use crate::core::uop::lower::support::FLOW_CASES;

    fn insn(program: &[u16]) -> CachedInsn {
        let mut core = core_with(program);
        let pc = core.pc;
        decode_at(&mut core, pc)
    }

    fn cx(cfg: &UopConfig, stop_pc: Option<u32>) -> LowerCtx<'_> {
        LowerCtx {
            cfg,
            stop_pc,
            simd: false,
            isa: false,
        }
    }

    #[test]
    fn flow_of_names_the_terminators_and_their_delay_slots() {
        for (name, program, want) in FLOW_CASES {
            assert_eq!(flow_of(&insn(program)), *want, "{name}");
        }
    }

    #[test]
    fn a_delayed_branch_lands_after_exactly_two_slots() {
        let mut c = core_with(&[0x063E, 0x0400, 0x0010, 0x0001, 0x0001, 0x0001]);
        let at = c.pc;
        let mut pcs = Vec::new();
        for _ in 0..3 {
            c.step().unwrap();
            pcs.push(c.pc);
        }
        assert_eq!(
            pcs,
            [at + 3, at + 4, 0x10],
            "branch, slot, slot, then the target"
        );
        assert_eq!(DELAY_SLOTS, 2);
    }

    #[test]
    fn only_loads_into_system_registers_touch_the_guard() {
        // R1 = 7, I0 = 7 (ureg 0x10), and a load into universal register 0x6A (a system register)
        for (u, want) in [
            (0x01u16, false),
            (0x10, false),
            (0x5F, false),
            (0x60, true),
            (0x6A, true),
        ] {
            assert_eq!(
                touches_sys(&insn(&[0x0F80 | u, 0x0007])),
                want,
                "ureg {u:#x}"
            );
        }
        assert!(!touches_sys(&insn(&[0x0001])));
        assert!(!touches_sys(&insn(&[0x013e, 0x0000, 0x1312])));
        assert!(
            touches_sys(&insn(&[0x1780, 0, 0])),
            "a stack operation is a cut as well"
        );
    }

    #[test]
    fn compute_classes_that_set_the_arithmetic_exception_flags_are_named() {
        let alu = insn(&[0x013e, 0x0000, 0x1312]);
        assert!(can_raise_arith(&alu));
        assert!(!can_raise_arith(&insn(&[0x0001])));
        assert!(!can_raise_arith(&insn(&[0x0F81, 0x0007])));
        for (op, want) in [
            (ComputeOp::Nop, false),
            (ComputeOp::Invalid, false),
            (
                ComputeOp::Shifter {
                    op: 0,
                    n: 0,
                    x: 0,
                    y: 0,
                },
                false,
            ),
            (
                ComputeOp::FixedAlu {
                    op: 0,
                    n: 0,
                    x: 0,
                    y: 0,
                },
                true,
            ),
            (ComputeOp::FloatMul { n: 0, x: 0, y: 0 }, true),
            (
                ComputeOp::FloatAlu {
                    op: 0x80,
                    n: 0,
                    x: 0,
                    y: 0,
                },
                true,
            ),
            (
                ComputeOp::FixedMult {
                    op: 0,
                    n: 0,
                    x: 0,
                    y: 0,
                },
                true,
            ),
            (ComputeOp::DualAddSub { c: 0 }, true),
            (ComputeOp::Multifunction { c: 0 }, true),
        ] {
            assert_eq!(compute_raises(&op), want, "{op:?}");
        }
    }

    #[test]
    fn unimplemented_instructions_are_not_translatable() {
        assert!(translatable(&insn(&[0x0001])));
        let mut bad = insn(&[0x0001]);
        bad.op = DecodedOp::Unimplemented;
        assert!(!translatable(&bad));
    }

    #[test]
    fn a_block_is_cut_by_flow_sys_loads_the_step_limit_and_stop_pc() {
        let cfg = UopConfig::default();
        let nop = insn(&[0x0001]);
        let plain = cx(&cfg, None);
        assert!(!cuts_after(&plain, &nop, 1));
        assert!(!cuts_after(&plain, &nop, cfg.max_block_steps - 1));
        assert!(
            cuts_after(&plain, &nop, cfg.max_block_steps),
            "max_block_steps"
        );
        assert!(cuts_after(&plain, &insn(&[0x0ABE]), 1), "flow");
        assert!(
            cuts_after(&plain, &insn(&[0x0F80 | 0x6A, 1]), 1),
            "system register load"
        );
        // the block that reaches stop_pc ends there, so stop_pc starts a block
        assert!(cuts_after(&cx(&cfg, Some(nop.next_pc)), &nop, 1));
        assert!(!cuts_after(&cx(&cfg, Some(nop.next_pc + 1)), &nop, 1));
        let one = UopConfig {
            max_block_steps: 0,
            ..cfg
        };
        assert!(
            cuts_after(&cx(&one, None), &nop, 1),
            "a zero limit still makes progress"
        );
    }
}
