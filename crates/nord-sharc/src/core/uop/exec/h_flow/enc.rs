//! Test-only encodings of the flow instructions, so the tests do not hand-write opcodes.
//!
//! OWNER: W2-FLOW. Layout, bit by bit, from `decode_cache.rs::FlowOp::decode` and the encodings of
//! the `branch.rs`, `branch_ext.rs` and `arith_loop.rs` tests (`lower::support::FLOW_CASES`). The
//! 48-bit word `w` holds the first parcel in bits 47..32; a 16-bit instruction is the same word
//! with the low parcels zero. Every builder is checked against the decode by
//! `every_builder_decodes_to_the_operation_it_names`: the parameters come back out of `FlowOp`.
use crate::core::decode_cache::{DecodedOp, FlowOp, MoveOp};
use crate::core::support::CODE;
use crate::core::uop::lower::decode::decode_at;

/// Condition code `TRUE` (always). VERIFIED: every builder decodes back to cond 31
/// (`every_builder_decodes_to_the_operation_it_names`).
pub(in crate::core) const TRUE: u8 = 31;

/// `IF cond JUMP/CALL target` with an absolute 24-bit target (Type 8a), 3 parcels.
pub(in crate::core) fn jump(cond: u8, target: u32, call: bool, delayed: bool) -> Vec<u16> {
    vec![
        0x0600 | u16::from(call) << 7 | u16::from(cond & 31) << 1,
        u16::from(delayed) << 10 | (target >> 16 & 0xFF) as u16,
        target as u16,
    ]
}

/// `IF cond JUMP/CALL (PC, offset)` with a 24-bit signed offset (Type 8a), 3 parcels.
pub(in crate::core) fn jump_rel(cond: u8, offset: i32, call: bool, delayed: bool) -> Vec<u16> {
    let mut p = jump(cond, offset as u32 & 0xFF_FFFF, call, delayed);
    p[0] |= 0x0100;
    p
}

/// The 6-bit field of bits 32..27 of a Type 9 word spread over its two parcels.
fn field6(p: &mut [u16], f: u8) {
    p[0] |= u16::from(f >> 5 & 1);
    p[1] |= u16::from(f & 0x1F) << 11;
}

/// `IF cond JUMP/CALL (Mm, Ii)` (Type 9b, 2 parcels); `reg_i` and `reg_m` are the DAG2 registers
/// 8..=15.
pub(in crate::core) fn jump_i(
    cond: u8,
    reg_i: u8,
    reg_m: u8,
    call: bool,
    delayed: bool,
) -> Vec<u16> {
    let mut p = vec![
        0x0800 | u16::from(call) << 7 | u16::from(cond & 31) << 1,
        0x003F | u16::from(delayed) << 10,
    ];
    field6(&mut p, (reg_i - 8) << 3 | (reg_m - 8));
    p
}

/// `IF cond JUMP/CALL (PC, offset)` with a 6-bit signed offset (Type 9b, 2 parcels).
pub(in crate::core) fn jump_pc6(cond: u8, offset: i8, call: bool, delayed: bool) -> Vec<u16> {
    let mut p = jump_i(cond, 8, 8, call, delayed);
    p[0] = p[0] & !1 | 0x0100;
    p[1] &= !(0x1F << 11);
    field6(&mut p, offset as u8 & 0x3F);
    p
}

/// `IF cond RTS` (16 bit, Type 11c) with the delayed and the interrupt-return bit.
pub(in crate::core) fn rts(cond: u8, delayed: bool, rti: bool) -> Vec<u16> {
    vec![0x0A80 | u16::from(rti) << 8 | u16::from(delayed) << 6 | u16::from(cond & 31) << 1]
}

/// `IF cond RTS/RTI` (48 bit, Type 11a) without a compute.
pub(in crate::core) fn ret_wide(cond: u8, delayed: bool, rti: bool) -> Vec<u16> {
    vec![
        0x0A00 | u16::from(rti) << 8 | u16::from(cond & 31) << 1,
        u16::from(delayed) << 10,
        0,
    ]
}

/// `DO (PC, end_off) UNTIL LCE` with an immediate count (Type 12a), 3 parcels.
pub(in crate::core) fn do_count(count: u16, end_off: u32) -> Vec<u16> {
    vec![
        0x0C00 | count >> 8,
        (count & 0xFF) << 8 | (end_off >> 16 & 0xFF) as u16,
        end_off as u16,
    ]
}

/// `DO (PC, end_off) UNTIL LCE` with the count in universal register `ureg` (Type 12a).
pub(in crate::core) fn do_reg(ureg: u8, end_off: u32) -> Vec<u16> {
    vec![
        0x0D00 | u16::from(ureg & 0x7F),
        (end_off >> 16 & 0xFF) as u16,
        end_off as u16,
    ]
}

/// `DO (PC, end_off) UNTIL cond` (Type 13a), 3 parcels.
pub(in crate::core) fn do_until(cond: u8, end_off: u32) -> Vec<u16> {
    vec![
        0x0E00 | u16::from(cond & 31) << 1,
        (end_off >> 16 & 0x7F) as u16,
        end_off as u16,
    ]
}

/// `IDLE`: 16 bit, or the 48-bit form.
pub(in crate::core) fn idle(wide: bool) -> Vec<u16> {
    if wide {
        vec![0x0080, 0, 0]
    } else {
        vec![0x0081]
    }
}

/// `ureg = value` (Type 17): 2 parcels when `value` is a sign-extended 16-bit immediate, else 3.
pub(in crate::core) fn load_imm(ureg: u8, value: u32) -> Vec<u16> {
    if i32::from(value as i16) as u32 == value {
        vec![0x0F80 | u16::from(ureg & 0x7F), value as u16]
    } else {
        vec![
            0x0F00 | u16::from(ureg & 0x7F),
            (value >> 16) as u16,
            value as u16,
        ]
    }
}

/// `NOP`. VERIFIED: decodes to `DecodedOp::Nop` (`lower::flow::chain`, `harness::tests`).
pub(in crate::core) const NOP: u16 = 0x0001;

/// A program under construction: parcels from `CODE`.
#[derive(Default)]
pub(in crate::core) struct Asm(pub Vec<u16>);

impl Asm {
    /// The pc of the next instruction.
    pub fn pc(&self) -> u32 {
        CODE + self.0.len() as u32
    }

    /// Appends one instruction and returns its pc.
    pub fn put(&mut self, p: &[u16]) -> u32 {
        let at = self.pc();
        self.0.extend_from_slice(p);
        at
    }

    /// Appends `n` NOPs.
    pub fn nops(&mut self, n: usize) {
        self.0.extend(std::iter::repeat_n(NOP, n));
    }

    /// Pads with NOPs up to `pc`.
    pub fn pad_to(&mut self, pc: u32) {
        let n = (pc - self.pc()) as usize;
        self.nops(n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    #[test]
    fn the_assembler_tracks_the_pc_pads_and_returns_where_an_instruction_went() {
        let mut a = Asm::default();
        assert_eq!(a.pc(), CODE);
        assert_eq!(a.put(&jump(TRUE, 0x100, false, false)), CODE);
        a.nops(2);
        assert_eq!(a.pc(), CODE + 5);
        a.pad_to(CODE + 8);
        assert_eq!((a.pc(), a.0.len(), a.0[7]), (CODE + 8, 8, NOP));
        assert_eq!(a.put(&[NOP]), CODE + 8);
    }

    fn decode(p: &[u16]) -> (DecodedOp, usize) {
        let mut c = core_with(p);
        let pc = c.pc;
        let i = decode_at(&mut c, pc);
        (i.op, (i.next_pc - pc) as usize)
    }

    fn flow(p: &[u16]) -> FlowOp {
        match decode(p) {
            (DecodedOp::Flow(f), len) => {
                assert_eq!(len, p.len(), "{p:04x?}: length");
                f
            }
            (op, _) => panic!("{p:04x?} decodes to {op:?}"),
        }
    }

    #[test]
    fn every_builder_decodes_to_the_operation_it_names() {
        for cond in [0u8, 7, 16, 30, 31] {
            for (call, delayed) in [(false, false), (true, false), (false, true), (true, true)] {
                assert_eq!(
                    flow(&jump(cond, 0x12_3456, call, delayed)),
                    FlowOp::JumpDirect {
                        cond,
                        target: 0x12_3456,
                        call,
                        delayed
                    }
                );
                let rel = flow(&jump_rel(cond, -5, call, delayed));
                assert_eq!(
                    rel,
                    FlowOp::JumpDirect {
                        cond,
                        target: CODE.wrapping_sub(5),
                        call,
                        delayed
                    }
                );
                let FlowOp::JumpIndirect {
                    cond: c,
                    reg_i,
                    reg_m,
                    rel_target,
                    call: k,
                    delayed: d,
                    compute,
                    ..
                } = flow(&jump_i(cond, 11, 14, call, delayed))
                else {
                    panic!("jump_i")
                };
                assert_eq!(
                    (c, reg_i, reg_m, rel_target, k, d, compute),
                    (cond, 11, 14, None, call, delayed, None)
                );
                for off in [-32i8, -1, 0, 5, 31] {
                    let FlowOp::JumpIndirect {
                        rel_target,
                        call: k,
                        delayed: d,
                        ..
                    } = flow(&jump_pc6(cond, off, call, delayed))
                    else {
                        panic!("jump_pc6")
                    };
                    let want = CODE.wrapping_add(i32::from(off) as u32);
                    assert_eq!(
                        (rel_target, k, d),
                        (Some(want), call, delayed),
                        "offset {off}"
                    );
                }
            }
            for (delayed, rti) in [(false, false), (true, false), (false, true), (true, true)] {
                for p in [rts(cond, delayed, rti), ret_wide(cond, delayed, rti)] {
                    let FlowOp::Return {
                        cond: c,
                        rti: r,
                        delayed: d,
                        compute,
                        ..
                    } = flow(&p)
                    else {
                        panic!("{p:04x?}")
                    };
                    assert_eq!((c, r, d, compute), (cond, rti, delayed, None), "{p:04x?}");
                }
            }
            let (op, len) = decode(&do_until(cond, 9));
            assert_eq!((op, len), (DecodedOp::DoUntil, 3));
        }
    }

    #[test]
    fn the_loop_idle_and_load_builders_carry_their_fields() {
        for (count, off) in [(1u16, 3u32), (300, 0x1_0005), (0xFFFF, 7)] {
            assert_eq!(
                flow(&do_count(count, off)),
                FlowOp::DoUntilCounter {
                    end: CODE + off,
                    count_ureg: None,
                    imm_count: u32::from(count)
                }
            );
        }
        let FlowOp::DoUntilCounter {
            count_ureg, end, ..
        } = flow(&do_reg(0x13, 0x42))
        else {
            panic!("do_reg")
        };
        assert_eq!((count_ureg, end), (Some(0x13), CODE + 0x42));
        assert_eq!(decode(&idle(false)), (DecodedOp::Idle, 1));
        assert_eq!(decode(&idle(true)), (DecodedOp::Idle, 3));
        for (ureg, value, len) in [
            (0x6Au8, 1u32, 2usize),
            (0x7B, 0xFFFF_FFFF, 2),
            (0x7B, 0xC_0430, 3),
            (0x01, 0x8000, 3),
        ] {
            let (op, l) = decode(&load_imm(ureg, value));
            assert_eq!(
                (op, l),
                (DecodedOp::Move(MoveOp::LoadImm { ureg, value }), len)
            );
        }
        assert_eq!(decode(&[NOP]), (DecodedOp::Nop, 1));
    }
}
