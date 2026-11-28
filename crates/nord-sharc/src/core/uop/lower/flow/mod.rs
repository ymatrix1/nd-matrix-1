//! Lowering of flow instructions to uops.
//!
//! OWNER: W2-FLOW. Jumps, calls, returns, delayed branches, `DO` loops, `IDLE` and the load of a
//! system register. The handlers are in `exec/h_flow/`; the operand layout is `h_flow::pack`.
//!
//! | Instruction | Uop | Block |
//! |---|---|---|
//! | `IF cond JUMP/CALL target` (absolute or PC-relative) | `JUMP` | ends, `slots` owed when delayed |
//! | `IF cond JUMP/CALL (PC, offset)` | `JUMP` (the target is known) | as above |
//! | `IF cond JUMP/CALL (Mm, Ii)` | `JUMP_I` | as above |
//! | `IF cond RTS/RTI` | `RET` | as above |
//! | `DO ... UNTIL LCE` | `DO_COUNT` | ends |
//! | `DO ... UNTIL cond` | `DO_UNTIL` | ends |
//! | `IDLE` | `IDLE` | ends |
//! | `sysreg = immediate` | `SYS_LOAD` | ends |
//!
//! Every uop accounts one step. A terminator runs `finish_step` itself (`h_flow/mod.rs`), so the
//! translator never puts `UF_FIN` on it, also when it is the last instruction of a loop body; the
//! delay slots and a system load that is a loop end or a slot do get it (`translate.rs`).
//!
//! What is left to `Interp` (exact by construction) and why:
//!
//! | Form | Reason |
//! |---|---|
//! | a jump with the `(LA)` or `(CI)` modifier | pops a loop frame or ends an interrupt service in the middle of the branch (`apply_jump_modifiers`) |
//! | a conditional branch with a compute field | the compute runs on both PEs with their own conditions (`on_each_pe`) |
//! | a delayed `RTI` | the interrupt bookkeeping pokes the deadline while the delay slots still follow in the block |
//! | `CJUMP`, `RFRAME`, `PUSH`/`POP` of a stack, `ureg = ureg` into a system register | other families or not worth a handler yet |
//! | any flow instruction while delay slots are owed | the translator never gets there (`group_fits`); declining keeps it so |
use super::builder::BlockBuilder;
use super::cut::touches_sys;
use super::{Family, LowerCtx};
use crate::core::decode_cache::{CachedInsn, DecodedOp, FlowOp, MoveOp};
use crate::core::uop::exec::h_flow::pack::{indirect, Branch, FROM_REG};
use crate::core::uop::exec::h_flow::{DO_COUNT, DO_UNTIL, IDLE, JUMP, JUMP_I, RET, SYS_LOAD};
use crate::core::uop::lower::cut::DELAY_SLOTS;
use crate::core::uop::types::Uop;
use crate::visa::bits;

#[cfg(test)]
mod chain;

/// The flow family.
pub(in crate::core) struct FlowFamily;

/// What the uop of one instruction does to the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// Ends the block, `slots` delay slots follow.
    Terminator { slots: u8 },
    /// Ends the block, no delay slot, not a branch (`SYS_LOAD`: the dispatch loop runs the
    /// `finish_step` of a slot or loop end).
    Sys,
}

/// The `(LA)` and `(CI)` modifiers of a jump (`branch_ext.rs::apply_jump_modifiers`): bit 39 is
/// the call bit, which has none; otherwise bit 38 pops a loop frame and bit 24 ends an interrupt.
fn has_jump_modifiers(c: &CachedInsn) -> bool {
    bits(c.w, 39, 39) == 0 && (bits(c.w, 38, 38) == 1 || bits(c.w, 24, 24) == 1)
}

/// The uop of `c` and its shape, or `None` when the instruction stays `Interp`.
fn uop_of(c: &CachedInsn) -> Option<(Uop, Shape)> {
    let len = c.next_pc.wrapping_sub(c.pc);
    if c.isa || !(1..=3).contains(&len) {
        return None;
    }
    let word = |delayed, call, rti| Branch::new(c.cond, len, delayed, call, rti).0;
    let slots = |delayed: bool| Shape::Terminator {
        slots: if delayed { DELAY_SLOTS } else { 0 },
    };
    let one = |kind, a, b| Uop::new(kind, 1, c.pc, a, b);
    match c.op {
        DecodedOp::Flow(FlowOp::JumpDirect {
            target,
            call,
            delayed,
            ..
        }) if !has_jump_modifiers(c) => Some((
            one(JUMP, target, word(delayed, call, false)),
            slots(delayed),
        )),
        DecodedOp::Flow(FlowOp::JumpIndirect {
            reg_i,
            reg_m,
            rel_target,
            call,
            delayed,
            compute: None,
            ..
        }) if !has_jump_modifiers(c) => {
            let b = word(delayed, call, false);
            let u = match rel_target {
                Some(target) => one(JUMP, target, b),
                None => one(JUMP_I, indirect(reg_i, reg_m), b),
            };
            Some((u, slots(delayed)))
        }
        DecodedOp::Flow(FlowOp::Return {
            rti,
            delayed,
            compute: None,
            ..
        }) if !(rti && delayed) => Some((one(RET, 0, word(delayed, false, rti)), slots(delayed))),
        DecodedOp::Flow(FlowOp::DoUntilCounter {
            end,
            count_ureg,
            imm_count,
        }) => {
            let count = count_ureg.map_or(imm_count, |r| FROM_REG | u32::from(r));
            Some((one(DO_COUNT, end, count), slots(false)))
        }
        DecodedOp::DoUntil => Some((one(DO_UNTIL, c.w as u32, (c.w >> 32) as u32), slots(false))),
        DecodedOp::Idle => Some((one(IDLE, 0, word(false, false, false)), slots(false))),
        DecodedOp::Move(MoveOp::LoadImm { ureg, value }) if touches_sys(c) => {
            Some((one(SYS_LOAD, u32::from(ureg), value), Shape::Sys))
        }
        _ => None,
    }
}

impl Family for FlowFamily {
    fn lower(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) -> bool {
        let _ = cx;
        let Some((u, shape)) = uop_of(c) else {
            return false;
        };
        match shape {
            Shape::Terminator { slots } => {
                if out.owed_slots() > 0 {
                    return false;
                }
                out.push(u);
                if slots > 0 {
                    out.owe_slots(slots);
                }
            }
            Shape::Sys => out.push(u),
        }
        out.end_block();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::lower::decode::decode_at;
    use crate::core::uop::lower::lower_first;
    use crate::core::uop::lower::support::{reg_move, FLOW_CASES, LOAD_SYS, NOP};
    use crate::core::uop::types::UopKind;

    /// The uop the family emits for `program`, with the builder state it leaves.
    fn lowered(program: &[u16]) -> Option<(Uop, bool, u8)> {
        let mut core = core_with(program);
        let pc = core.pc;
        let c = decode_at(&mut core, pc);
        let cfg = crate::core::uop::config::UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd: false,
            isa: false,
        };
        let mut b = BlockBuilder::new(pc, false, false);
        FlowFamily::lower(&cx, &c, &mut b).then(|| (b.uops()[0], b.ends(), b.owed_slots()))
    }

    fn kind(program: &[u16]) -> Option<UopKind> {
        lowered(program).map(|(u, _, _)| UopKind(u.h))
    }

    #[test]
    fn each_flow_form_gets_its_kind_and_ends_the_block() {
        let want: &[(&str, UopKind, u8)] = &[
            ("jump", JUMP, 0),
            ("jump db", JUMP, 2),
            ("jump rel", JUMP, 0),
            ("jump rel db", JUMP, 2),
            ("if eq jump", JUMP, 0),
            ("call", JUMP, 0),
            ("call db", JUMP, 2),
            ("jump (m,i)", JUMP_I, 0),
            ("jump (m,i) db", JUMP_I, 2),
            ("call (m,i)", JUMP_I, 0),
            ("rts", RET, 0),
            ("rts db", RET, 2),
            ("rts wide db", RET, 2),
            ("do lce imm", DO_COUNT, 0),
            ("do lce reg", DO_COUNT, 0),
            ("do until", DO_UNTIL, 0),
            ("do forever", DO_UNTIL, 0),
            ("idle", IDLE, 0),
        ];
        for (name, k, slots) in want {
            let (_, program, _) = FLOW_CASES.iter().find(|(n, ..)| n == name).unwrap();
            let (u, ends, owed) = lowered(program).unwrap_or_else(|| panic!("{name} declined"));
            assert_eq!((UopKind(u.h), ends, owed), (*k, true, *slots), "{name}");
            assert_eq!(
                (u.steps, u.pc, u.flags),
                (1, CODE, 0),
                "{name}: one step, no flag"
            );
        }
    }

    #[test]
    fn the_operands_carry_the_target_the_condition_and_the_length() {
        let (u, ..) = lowered(&[0x0600, 0x0000, 0x0010]).unwrap(); // IF EQ JUMP 0x10
        let b = Branch(u.b);
        assert_eq!(
            (u.a, b.cond(), b.len(), b.delayed(), b.call()),
            (0x10, 0, 3, false, false)
        );
        let (u, ..) = lowered(&[0x06BE, 0x0400, 0x0010]).unwrap(); // CALL 0x10 (DB)
        let b = Branch(u.b);
        assert_eq!(
            (b.cond(), b.delayed(), b.call(), b.rti()),
            (31, true, true, false)
        );
        let (u, ..) = lowered(&[0x073E, 0x0000, 0x0004]).unwrap(); // JUMP (PC, 4)
        assert_eq!(
            u.a,
            CODE + 4,
            "relative: the target is resolved at translation time"
        );
        let (u, ..) = lowered(&[0x0ABE]).unwrap(); // RTS, a 16-bit instruction
        assert_eq!(Branch(u.b).len(), 1);
        let (u, ..) = lowered(&[0x0B3E, 0x0000, 0x0000]).unwrap(); // RTI
        assert!(Branch(u.b).rti() && !Branch(u.b).delayed());
        let (u, ..) = lowered(&[0x0C00, 0x0500, 0x0006]).unwrap(); // DO (PC, 6) UNTIL LCE, 5
        assert_eq!((u.a, u.b), (CODE + 6, 5));
        let (u, ..) = lowered(&[0x0D00 | 3, 0x0000, 0x0003]).unwrap(); // LCNTR = R3
        assert_eq!((u.a, u.b), (CODE + 3, FROM_REG | 3));
        let (u, ..) = lowered(&[0x0E00, 0x0000, 0x0007]).unwrap();
        let w = 0x0E00_0000_0007u64;
        assert_eq!((u.a, u.b), (w as u32, (w >> 32) as u32));
        let (u, ..) = lowered(&[0x083E, 0x003F | 5 << 11 | 2 << 14]).unwrap(); // (M13, I10)
        assert_eq!(u.a, indirect(8 + 2, 8 + 5));
    }

    #[test]
    fn a_system_register_load_is_a_sys_load_and_any_other_register_load_is_not_taken() {
        let (u, ends, owed) = lowered(&LOAD_SYS).unwrap();
        assert_eq!(
            (UopKind(u.h), u.a, u.b, ends, owed),
            (SYS_LOAD, 0x6A, 1, true, 0)
        );
        assert_eq!(kind(&[0x0F81, 0x0007]), None, "R1 = 7 is a move");
        assert_eq!(
            kind(&[0x0F80 | 0x5F, 0x0007]),
            None,
            "the last non-system code"
        );
        assert_eq!(
            kind(&[0x0F80 | 0x60, 0xFFFF]).map(|k| k.0),
            Some(SYS_LOAD.0)
        );
        let (u, ..) = lowered(&[0x0F80 | 0x7B, 0xFFFF]).unwrap();
        assert_eq!(
            u.b, 0xFFFF_FFFF,
            "the 16-bit immediate is sign-extended by the decode"
        );
        assert_eq!(
            kind(&reg_move(0x01, 0x6A, true)),
            None,
            "ureg = ureg is not taken"
        );
    }

    #[test]
    fn what_the_handlers_do_not_model_stays_with_the_interpreter() {
        // (LA) on a non-call jump (bit 38), (CI) (bit 24), both only without the call bit
        assert_eq!(kind(&[0x063E | 0x0040, 0x0000, 0x0010]), None, "JUMP (LA)");
        assert_eq!(kind(&[0x063E, 0x0100, 0x0010]), None, "JUMP (CI)");
        assert!(
            kind(&[0x06BE | 0x0040, 0x0000, 0x0010]).is_some(),
            "a call has no modifier"
        );
        // a delayed RTI, a conditional compute (IF cond RTS, compute)
        assert_eq!(kind(&[0x0B3E, 0x0400, 0x0000]), None, "RTI (DB)");
        assert_eq!(kind(&[0x0A3E, 0x0000, 0x1312]), None, "RTS with a compute");
        assert_eq!(kind(&[0x1800, 0x0000, 0x1234]), None, "CJUMP");
        assert_eq!(kind(&[0x1708]), None, "PUSH PCSTK");
        assert_eq!(kind(&[0x1901]), None, "RFRAME");
        assert_eq!(kind(&[NOP]), None);
        assert_eq!(kind(&[0x013e, 0x0000, 0x1312]), None, "R3 = R1 + R2");
    }

    #[test]
    fn a_terminator_is_declined_while_delay_slots_are_owed_and_a_system_load_is_not() {
        let mut core = core_with(&[0x063E, 0x0000, 0x0010]);
        let pc = core.pc;
        let jump = decode_at(&mut core, pc);
        let cfg = crate::core::uop::config::UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd: false,
            isa: false,
        };
        let mut b = BlockBuilder::new(pc, false, false);
        b.owe_slots(2);
        assert!(!FlowFamily::lower(&cx, &jump, &mut b));
        assert_eq!(b.len(), 0, "nothing emitted");
        crate::core::support::load(&mut core, pc, &LOAD_SYS);
        core.invalidate_code_cache();
        let sys = decode_at(&mut core, pc);
        assert!(FlowFamily::lower(&cx, &sys, &mut b));
        assert_eq!(
            (b.len(), b.owed_slots()),
            (1, 2),
            "the slot count is the translator's"
        );
    }

    #[test]
    fn lower_first_reports_the_family_take_and_one_uop_for_a_jump_and_nothing_for_a_nop() {
        assert_eq!(lower_first::<FlowFamily>(&[0x0001]), (false, 0));
        assert_eq!(
            lower_first::<FlowFamily>(&[0x063E, 0x0000, 0x0010]),
            (true, 1)
        );
    }
}
