//! Lowering of float instructions to uops.
//!
//! OWNER: W2-FLOAT. A float compute instruction (`IF cond` conditional, unconditional and the 16-bit
//! short form: `DecodedOp::ComputeCond` and `ComputeBoth`) becomes ONE uop of one step, whose
//! handler (`exec/h_float`) runs it on PEx and, in a block translated for PEy on, on PEy:
//!
//! | Instruction | Kind |
//! |---|---|
//! | float multiply | `FMUL` |
//! | `+`, `-`, average, compare | `FADD`, `FSUB`, `FAVG`, `FCMP` |
//! | `COPYSIGN`, `MIN`, `MAX`, `CLIP` | `FSEL` |
//! | `PASS`, `-`, `ABS` | `FABSNEG` |
//! | `RND`, `SCALB`, `FIX`, `TRUNC`, `FLOAT` | `FCONV` |
//! | `RECIPS`, `RSQRTS`, float multiplier + ALU multifunction | `FGEN` (through `Pe::exec`) |
//!
//! Everything else stays an `Interp` uop (exact by construction): the other float opcodes
//! (`LOGB`, `MANT`, the absolute-sum forms), every fixed-point, shifter and multiplier op, and an
//! undecodable compute field. The condition code goes to `Uop::b`; `UF_SIMD` is set exactly when
//! `LowerCtx::simd` is.
//!
//! One rule the translator cannot know: a float instruction in the FIRST delay slot of a delayed
//! branch is declined (`BlockBuilder::owed_slots() > 1`). Its uop would carry `UF_FIN` and not be
//! the last of the block, and a float uop that raised an arithmetic exception has to end the block
//! after it (`exec/h_float/run.rs::poked`), which a `finish_step` still owed by the driver would
//! not allow. The second slot is the last uop of the group and is taken. Float computes in the
//! first slot of a delayed branch are rare.
//!
//! Provenance: VERIFIED by `lower_takes_exactly_the_float_instructions_of_the_census` (every
//! float instruction of the committed census is taken, nothing else is) and by the differential
//! tests of `exec/h_float` (the uop is equal to the interpreter step).
use super::builder::BlockBuilder;
use super::{Family, LowerCtx};
use crate::core::decode_cache::{CachedInsn, ComputeOp, DecodedOp};
use crate::core::uop::exec::h_float::{
    conv_opcode, fgen_supports, fgen_word, pack, FABSNEG, FADD, FAVG, FCMP, FCONV, FGEN, FMUL,
    FSEL, FSUB,
};
use crate::core::uop::types::{Uop, UopKind, UF_SIMD};

#[cfg(test)]
mod census;

/// The float family.
pub(in crate::core) struct FloatFamily;

/// The kind and `Uop::a` of float compute `op`, `None` when it stays an `Interp` uop.
pub(in crate::core) fn select(op: ComputeOp) -> Option<(UopKind, u32)> {
    let uop = |kind, n, x, y, p| Some((kind, pack(n, x, y, p)));
    match op {
        ComputeOp::FloatMul { n, x, y } => uop(FMUL, n, x, y, 0),
        ComputeOp::FloatAlu { op: o, n, x, y } => match o {
            0x81 => uop(FADD, n, x, y, 0),
            0x82 => uop(FSUB, n, x, y, 0),
            0x89 => uop(FAVG, n, x, y, 0),
            0x8A => uop(FCMP, 0, x, y, 0),
            0xA1 => uop(FABSNEG, n, x, 0, 0),
            0xA2 => uop(FABSNEG, n, x, 0, 1),
            0xB0 => uop(FABSNEG, n, x, 0, 2),
            0xE0..=0xE3 => uop(FSEL, n, x, y, u32::from(o & 3)),
            o if conv_opcode(u32::from(o)) => uop(FCONV, n, x, y, u32::from(o)),
            _ => fgen_word(op).map(|w| (FGEN, w)),
        },
        ComputeOp::Multifunction { .. } if fgen_supports(op) => fgen_word(op).map(|w| (FGEN, w)),
        _ => None,
    }
}

impl Family for FloatFamily {
    fn lower(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) -> bool {
        let (cond, op) = match c.op {
            DecodedOp::ComputeCond { cond, op } => (cond, op),
            DecodedOp::ComputeBoth { op } => (31, op),
            _ => return false,
        };
        if cond > 31 || out.owed_slots() > 1 {
            return false;
        }
        let Some((kind, a)) = select(op) else {
            return false;
        };
        let mut uop = Uop::new(kind, 1, c.pc, a, u32::from(cond));
        if cx.simd {
            uop.flags |= UF_SIMD;
        }
        out.push(uop);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::exec::h_float::{unpack, KINDS};
    use crate::core::uop::lower::decode::decode_at;
    use crate::core::uop::lower::lower_first;
    use crate::core::uop::types::KindFamily;

    /// `IF cond <compute>` as a Type 2a instruction: parcels `0x0100 | cond << 1`, the high seven
    /// bits of the 23-bit compute field and its low 16 bits (the layout of `[0x013e, 0, 0x1312]`).
    fn t2a(cond: u16, compute: u32) -> [u16; 3] {
        [
            0x0100 | cond << 1,
            (compute >> 16) as u16 & 0x7F,
            compute as u16,
        ]
    }

    fn alu(op: u32, n: u32, x: u32, y: u32) -> u32 {
        op << 12 | n << 8 | x << 4 | y
    }

    /// The lowering of one instruction: whether it was taken and the uops.
    fn lower_one(parcels: &[u16], simd: bool) -> (bool, Vec<Uop>) {
        let mut c = core_with(parcels);
        let pc = c.pc;
        let insn = decode_at(&mut c, pc);
        let cfg = UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd,
            isa: false,
        };
        let mut b = BlockBuilder::new(pc, simd, false);
        let taken = FloatFamily::lower(&cx, &insn, &mut b);
        (taken, b.uops().to_vec())
    }

    #[test]
    fn a_float_add_is_one_uop_with_its_operands_condition_and_simd_flag() {
        // IF NE F3 = F1 + F2 (condition 16), compute opcode 0x81
        let p = t2a(16, alu(0x81, 3, 1, 2));
        let (taken, uops) = lower_one(&p, false);
        assert!(taken);
        let [u] = uops[..] else { panic!("one uop") };
        assert_eq!(
            (u.h, u.steps, u.flags, u.b, u.pc),
            (FADD.0, 1, 0, 16, 0x12_0000)
        );
        assert_eq!(
            unpack(u.a),
            crate::core::uop::exec::h_float::Args {
                n: 3,
                x: 1,
                y: 2,
                p: 0
            }
        );
        let (_, simd) = lower_one(&p, true);
        assert_eq!(simd[0].flags, UF_SIMD, "the PEy-on variant carries UF_SIMD");
        assert_eq!(UopKind(simd[0].h).family(), Some(KindFamily::Float));
    }

    #[test]
    fn every_kind_has_a_lowering_and_the_operand_conventions_hold() {
        // (compute field, kind, parameter)
        let cases = [
            (alu(0x81, 4, 5, 6), FADD, 0),
            (alu(0x82, 4, 5, 6), FSUB, 0),
            (alu(0x89, 4, 5, 6), FAVG, 0),
            (alu(0x8A, 4, 5, 6), FCMP, 0),
            (alu(0xA1, 4, 5, 6), FABSNEG, 0),
            (alu(0xA2, 4, 5, 6), FABSNEG, 1),
            (alu(0xB0, 4, 5, 6), FABSNEG, 2),
            (alu(0xE0, 4, 5, 6), FSEL, 0),
            (alu(0xE3, 4, 5, 6), FSEL, 3),
            (alu(0xC9, 4, 5, 6), FCONV, 0xC9),
            (alu(0xDA, 4, 5, 6), FCONV, 0xDA),
            (alu(0xA5, 4, 5, 6), FCONV, 0xA5),
            (alu(0xBD, 4, 5, 6), FCONV, 0xBD),
            (alu(0xC4, 4, 5, 6), FGEN, 0),
            (alu(0xC5, 4, 5, 6), FGEN, 0),
            (1 << 20 | 0x30 << 12 | 4 << 8 | 5 << 4 | 6, FMUL, 0),
        ];
        let mut kinds = std::collections::BTreeSet::new();
        for (compute, kind, p) in cases {
            let (taken, uops) = lower_one(&t2a(31, compute), false);
            assert!(taken, "{compute:#x}");
            assert_eq!(uops[0].h, kind.0, "{compute:#x}");
            kinds.insert(kind.0);
            if kind != FGEN {
                let a = unpack(uops[0].a);
                assert_eq!(a.p, p, "{compute:#x}");
                assert_eq!(a.n, if kind == FCMP { 0 } else { 4 }, "{compute:#x}");
                assert_eq!(a.x, 5, "{compute:#x}");
                assert_eq!(a.y, if kind == FABSNEG { 0 } else { 6 }, "{compute:#x}");
            }
        }
        let all: std::collections::BTreeSet<u16> = KINDS.iter().map(|k| k.0).collect();
        assert_eq!(kinds, all, "a lowering for every kind of the range");
    }

    #[test]
    fn what_the_float_family_does_not_run_stays_an_interp_uop() {
        // fixed-point ALU, shifter, nop, a float opcode with no handler (LOGB 0xC1), an invalid
        // compute field, a move, a jump
        let declined: [&[u16]; 6] = [
            &[0x013e, 0x0000, 0x1312],
            &[0x0001],
            &[0x0F80, 0x0001],
            &[0x063E, 0x0000, 0x0010],
            &t2a(31, alu(0xC1, 1, 2, 3)),
            &t2a(31, 3 << 20 | 0x12345),
        ];
        for p in declined {
            assert_eq!(lower_first::<FloatFamily>(p), (false, 0), "{p:x?}");
        }
        // a fixed multifunction (bit 22 set, unit bits 21..19 = 0) is not a float one
        let fixed_multi = t2a(31, 1 << 22 | 0x1234);
        assert_eq!(lower_first::<FloatFamily>(&fixed_multi), (false, 0));
        // a condition code above 31 cannot be built from a 5-bit field, so only a hand-made
        // `CachedInsn` reaches the guard: it is declined and emits nothing
        let mut c = core_with(&t2a(31, alu(0x81, 1, 2, 3)));
        let pc = c.pc;
        let mut insn = decode_at(&mut c, pc);
        insn.op = DecodedOp::ComputeCond {
            cond: 32,
            op: ComputeOp::FloatMul { n: 1, x: 2, y: 3 },
        };
        let cfg = UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd: false,
            isa: false,
        };
        let mut b = BlockBuilder::new(pc, false, false);
        assert!(!FloatFamily::lower(&cx, &insn, &mut b) && b.is_empty());
    }

    #[test]
    fn a_float_instruction_in_the_first_delay_slot_is_declined_the_second_is_taken() {
        let mut c = core_with(&t2a(31, alu(0x81, 1, 2, 3)));
        let pc = c.pc;
        let insn = decode_at(&mut c, pc);
        let cfg = UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd: false,
            isa: false,
        };
        for (owed, taken) in [(0u8, true), (1, true), (2, false)] {
            let mut b = BlockBuilder::new(pc, false, false);
            b.owe_slots(owed);
            assert_eq!(FloatFamily::lower(&cx, &insn, &mut b), taken, "owed {owed}");
            assert_eq!(b.len(), usize::from(taken));
        }
    }
}
