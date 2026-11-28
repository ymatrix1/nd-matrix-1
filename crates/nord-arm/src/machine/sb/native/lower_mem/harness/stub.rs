//! OWNER: W2-NMEM. The test-only stand-in for the data-processing lowerer (W2-NDP owns the real
//! one): just the uops the memory and flow traces need.

use super::super::super::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::{k, Uop};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder};

/// The few data-processing uops the traces need, lowered the plain way (non-S moves and adds,
/// the compares with the usual `N Z C V` of `rn - op`); anything else is `Unsupported`.
pub(crate) fn dp_stub(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    match u.kind {
        k::ADD_IMM | k::SUB_IMM => {
            let a = cx.reg(u.rn);
            let imm = i64::from(u.imm as i32);
            let v = if u.kind == k::ADD_IMM {
                cx.b.ins().iadd_imm(a, imm)
            } else {
                cx.b.ins().iadd_imm(a, -imm)
            };
            cx.set_reg(u.rd, v);
        }
        k::MOV_IMM => {
            let v = cx.const32(u.imm);
            cx.set_reg(u.rd, v);
        }
        k::CMP_IMM | k::CMP_REG => {
            let a = cx.reg(u.rn);
            let b = if u.kind == k::CMP_IMM {
                cx.const32(u.imm)
            } else {
                cx.reg(u.rm)
            };
            let res = cx.b.ins().isub(a, b);
            let n = cx.b.ins().ushr_imm(res, 31);
            let z = cx.b.ins().icmp_imm(IntCC::Equal, res, 0);
            let z = cx.b.ins().uextend(types::I32, z);
            let c = cx.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, a, b);
            let c = cx.b.ins().uextend(types::I32, c);
            let (x, y) = (cx.b.ins().bxor(a, b), cx.b.ins().bxor(a, res));
            let v = cx.b.ins().band(x, y);
            let v = cx.b.ins().ushr_imm(v, 31);
            cx.set_flag(3, n);
            cx.set_flag(2, z);
            cx.set_flag(1, c);
            cx.set_flag(0, v);
        }
        _ => return Err(Unsupported),
    }
    Ok(())
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn the_stub_lowers_the_few_dp_kinds_and_refuses_the_rest() {
        let Some(mut jit) = super::super::native_or_skip("the_stub_lowers") else {
            return;
        };
        let _ = &mut jit;
        assert_eq!(k::family(k::ADD_IMM), k::Family::Dp);
    }
}
