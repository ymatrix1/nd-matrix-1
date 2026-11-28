//! OWNER: W2-NDP. The specialised data-processing ids `96..=107` (`MOV_IMM` to `ORR_IMM`): the
//! generic id they replace without the shifter, as `exec_alu::special` defines them.
//!
//! | Id | Native |
//! |---|---|
//! | `MOV_IMM`, `MVN_IMM` | `rd = imm`, `rd = !imm` (constants) |
//! | `MOV_REG` | `rd = rm` |
//! | `ADD_IMM`, `SUB_IMM`, `ORR_IMM` | `rd = rn + imm`, `rn - imm`, `rn \| imm` |
//! | `ADD_REG`, `SUB_REG` | `rd = rn + rm`, `rn - rm` |
//! | `CMP_IMM`, `CMP_REG` | N Z C V of `rn - imm`, `rn - rm`; no register |
//! | `TST_IMM` | N Z of `rn & imm`, C is the old C when `sh` bit 0 is set else bit 31 of `imm`, V kept |
//! | `TST_REG` | N Z of `rn & rm`, C and V kept (`lsl #0`: the shifter carry is the old C) |
//!
//! A uop that names `r15` in a field it uses is [`Unsupported`] (I2), decided before anything
//! is emitted.
//!
//! Provenance: VERIFIED against `exec_alu::exec_dp` by `lower_dp/diff.rs` (all 12 ids on random
//! states, per-id exercised counts) and against the generic id each replaces; the mutants 808,
//! 811 to 813, 822 to 824 of `lower_dp/kill.rs` are killed by them.

use super::{arith, mutant, set_nz, used};
use crate::machine::sb::native::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::{k, Uop};
use cranelift_codegen::ir::InstBuilder;

/// Emit the specialised uop `u` (`96..=107`).
pub(super) fn special(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    let (rd, rn, rm) = (u.rd, u.rn, u.rm);
    let imm = i64::from(u.imm as i32);
    match u.kind {
        k::MOV_IMM => {
            used(&[rd])?;
            let v = cx.const32(u.imm);
            cx.set_reg(rd, v);
        }
        k::MVN_IMM => {
            used(&[rd])?;
            let v = cx.const32(if mutant(812) { u.imm } else { !u.imm });
            cx.set_reg(rd, v);
        }
        k::MOV_REG => {
            used(&[rd, rm])?;
            let v = cx.reg(rm);
            cx.set_reg(rd, v);
        }
        k::ADD_IMM => {
            used(&[rd, rn])?;
            let a = cx.reg(rn);
            let v = cx.b.ins().iadd_imm(a, imm);
            cx.set_reg(rd, v);
        }
        k::SUB_IMM => {
            used(&[rd, rn])?;
            let a = cx.reg(rn);
            let v = if mutant(822) {
                cx.b.ins().iadd_imm(a, imm)
            } else {
                let c = cx.const32(u.imm);
                cx.b.ins().isub(a, c)
            };
            cx.set_reg(rd, v);
        }
        k::ORR_IMM => {
            used(&[rd, rn])?;
            let a = cx.reg(rn);
            let v = if mutant(823) {
                cx.b.ins().band_imm(a, imm)
            } else {
                cx.b.ins().bor_imm(a, imm)
            };
            cx.set_reg(rd, v);
        }
        k::ADD_REG => {
            used(&[rd, rn, rm])?;
            let a = cx.reg(rn);
            let b = if mutant(824) { a } else { cx.reg(rm) };
            let v = cx.b.ins().iadd(a, b);
            cx.set_reg(rd, v);
        }
        k::SUB_REG => {
            used(&[rd, rn, rm])?;
            let (a, b) = (cx.reg(rn), cx.reg(rm));
            let v = cx.b.ins().isub(a, b);
            cx.set_reg(rd, v);
        }
        k::CMP_IMM => {
            used(&[rn])?;
            let a = cx.reg(rn);
            let b = cx.const32(u.imm);
            let op = if mutant(811) { 4 } else { 10 };
            arith::arith(cx, op, a, b, true);
        }
        k::CMP_REG => {
            used(&[rn, rm])?;
            let (a, b) = (cx.reg(rn), cx.reg(rm));
            arith::arith(cx, 10, a, b, true);
        }
        k::TST_IMM => {
            used(&[rn])?;
            let a = cx.reg(rn);
            let res = cx.b.ins().band_imm(a, imm);
            set_nz(cx, res);
            if u.sh & 1 == 0 {
                let carry = cx.const32(u.imm >> 31);
                cx.set_flag(1, carry);
            }
        }
        k::TST_REG => {
            used(&[rn, rm])?;
            let (a, b) = (cx.reg(rn), cx.reg(rm));
            let res = cx.b.ins().band(a, b);
            set_nz(cx, res);
            if mutant(813) {
                let zero = cx.const32(0);
                cx.set_flag(1, zero);
            }
        }
        _ => return Err(Unsupported),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::native::emit::testing::build_and_verify;

    #[test]
    fn each_specialised_id_builds_verified_ir_with_the_register_and_flag_effects_of_its_row() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        // (kind, registers written, flag bits written)
        let rows: [(u8, u16, u8); 12] = [
            (k::MOV_IMM, 1 << 3, 0),
            (k::MOV_REG, 1 << 3, 0),
            (k::MVN_IMM, 1 << 3, 0),
            (k::ADD_IMM, 1 << 3, 0),
            (k::SUB_IMM, 1 << 3, 0),
            (k::ADD_REG, 1 << 3, 0),
            (k::SUB_REG, 1 << 3, 0),
            (k::CMP_IMM, 0, 0b1111),
            (k::CMP_REG, 0, 0b1111),
            (k::TST_IMM, 0, 0b1110),
            (k::TST_REG, 0, 0b1100),
            (k::ORR_IMM, 1 << 3, 0),
        ];
        for (kind, regs, flags) in rows {
            build_and_verify(&sb, |cx| {
                let mut u = Uop::new(kind, 0x1000);
                (u.rd, u.rn, u.rm, u.imm) = (3, 1, 2, 0x8000_00FF);
                assert_eq!(special(cx, &u), Ok(()), "kind {kind}");
                assert_eq!((cx.dirty, cx.flags_def), (regs, flags), "kind {kind}");
                cx.exit_generic(0x1000);
            });
        }
        build_and_verify(&sb, |cx| {
            let mut u = Uop::new(k::TST_IMM, 0x1000);
            (u.rn, u.imm, u.sh) = (1, 0x8000_0000, 1);
            assert_eq!(special(cx, &u), Ok(()));
            assert_eq!(cx.flags_def, 0b1100, "rotation 0: C is the old C");
            cx.exit_generic(0x1000);
        });
    }
}
