//! OWNER: W2-NDP. The second operand of a data-processing uop and its shifter carry, as SSA
//! values (the native image of `exec_alu::operand` and `alu::shift`).
//!
//! | Form | Operand | Shifter carry |
//! |---|---|---|
//! | 0 | the rotated immediate `u.imm` (a constant) | `sh` bit 0 set (rotation 0): the old C; else bit 31 of `imm` (a constant) |
//! | 1 | `Rm` shifted by `sh & 31` of type `sh >> 5 & 3`, all resolved at lowering time (LSR/ASR #0 is 32, ROR #0 is RRX) | by type and amount; LSL #0 keeps C |
//! | 2 | `Rm` shifted by `r[rs] & 0xFF` of type `sh >> 5 & 3`, resolved with selects | amount 0 keeps C; LSL/LSR by 32 the edge bit; by more than 32 zero; ASR by 32 or more bit 31; ROR by a multiple of 32 bit 31 |
//!
//! Cranelift masks a shift amount to 5 bits (`ishl`, `ushr`, `sshr`, `rotr` on `i32`), so the
//! register forms compute the in-range value with the masked amount and select the out-of-range
//! rule (amount 32 and above) explicitly. A carry of `None` means "unchanged": the caller writes
//! no C flag. The carry is built only when `want_carry` is set (S forms of logic ops).
//!
//! Provenance: VERIFIED against `alu::shift` (through the tier-2 `exec_alu::exec_dp`) by the
//! differential tests of `lower_dp/diff.rs`: every type x amount 0..=31 of form 1 and every
//! amount 0..=300 with noise in the high bits of form 2, each on 13 edge values x 16 flag
//! nibbles, zero differences; the mutants 804..=806, 810 and 815..=818, 825 of
//! `lower_dp/kill.rs` are all killed by them.

use super::{bit, bit_dyn, mutant};
use crate::machine::sb::native::emit::NCtx;
use crate::machine::sb::uop::Uop;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{InstBuilder, Value};

/// The operand value and, when asked for and not simply C, its shifter carry (an `i32` 0 or 1).
pub(super) struct Operand {
    pub value: Value,
    pub carry: Option<Value>,
}

/// The second operand of `u` in operand form `form` (0, 1 or 2).
pub(super) fn operand(cx: &mut NCtx<'_>, u: &Uop, form: u32, want_carry: bool) -> Operand {
    match form {
        0 => immediate(cx, u),
        1 => by_immediate(cx, u, want_carry),
        _ => by_register(cx, u, want_carry),
    }
}

fn immediate(cx: &mut NCtx<'_>, u: &Uop) -> Operand {
    let value = cx.const32(u.imm);
    let keep = u.sh & 1 != 0 && !mutant(804);
    let carry = (!keep).then(|| cx.const32(u.imm >> 31));
    Operand { value, carry }
}

/// Register shifted by an immediate: the amount is static, so each case is a fixed sequence.
/// `edge` is the bit of `Rm` that becomes the carry.
fn by_immediate(cx: &mut NCtx<'_>, u: &Uop, want: bool) -> Operand {
    let (ty, amount) = (u.sh >> 5 & 3, u32::from(u.sh & 31));
    let rm = cx.reg(u.rm);
    let n = i64::from(amount);
    let (value, edge) = match (ty, amount) {
        (0, 0) => {
            return Operand {
                value: rm,
                carry: None,
            }
        }
        (1 | 2, 0) if mutant(806) => {
            return Operand {
                value: rm,
                carry: None,
            }
        }
        (0, _) => (cx.b.ins().ishl_imm(rm, n), 32 - amount),
        (1, 0) => (cx.const32(0), 31),
        (1, _) => (cx.b.ins().ushr_imm(rm, n), amount - 1),
        (2, 0) => (cx.b.ins().sshr_imm(rm, 31), 31),
        (2, _) => (cx.b.ins().sshr_imm(rm, n), amount - 1),
        (_, 0) => {
            // RRX: the old C comes in at bit 31, bit 0 goes out
            let old = if mutant(810) {
                cx.const32(0)
            } else {
                cx.flag(1)
            };
            let hi = cx.b.ins().ishl_imm(old, 31);
            let lo = cx.b.ins().ushr_imm(rm, 1);
            (cx.b.ins().bor(hi, lo), 0)
        }
        (_, _) => (cx.b.ins().rotr_imm(rm, n), amount - 1),
    };
    let carry = want.then(|| bit(cx, rm, edge));
    Operand { value, carry }
}

/// Register shifted by a register: the low byte of `Rs`, resolved with selects. The value for
/// amount 0 is `Rm` in every arm (a shift or rotate by 0), and amount 0 keeps C.
fn by_register(cx: &mut NCtx<'_>, u: &Uop, want: bool) -> Operand {
    let ty = u.sh >> 5 & 3;
    let rm = cx.reg(u.rm);
    let rs = cx.reg(u.rs);
    let mask = match () {
        () if mutant(805) => 31,
        () if mutant(825) => 0x1FF,
        () => 0xFF,
    };
    let amt = cx.b.ins().band_imm(rs, mask);
    let zero = cx.const32(0);
    let big =
        cx.b.ins()
            .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, amt, 32);
    // (value, edge bit of the carry, the amounts whose carry is that bit: `Some(cc)` means
    // `amt cc 32`, `None` every non-zero amount)
    let (value, raw, range) = match ty {
        0 => {
            let v = cx.b.ins().ishl(rm, amt);
            let v = cx.b.ins().select(big, zero, v);
            let k = cx.b.ins().irsub_imm(amt, 32);
            let cc = if mutant(815) {
                IntCC::UnsignedLessThan
            } else {
                IntCC::UnsignedLessThanOrEqual
            };
            (v, want.then(|| bit_dyn(cx, rm, k)), Some(cc))
        }
        1 => {
            let past = if mutant(818) {
                cx.b.ins().icmp_imm(IntCC::UnsignedGreaterThan, amt, 32)
            } else {
                big
            };
            let v = cx.b.ins().ushr(rm, amt);
            let v = cx.b.ins().select(past, zero, v);
            let k = cx.b.ins().iadd_imm(amt, -1);
            (
                v,
                want.then(|| bit_dyn(cx, rm, k)),
                Some(IntCC::UnsignedLessThanOrEqual),
            )
        }
        2 => {
            let top = cx.const32(31);
            let low = cx.b.ins().umin(amt, top);
            let v = cx.b.ins().sshr(rm, low);
            let c32 = cx.const32(32);
            let m32 = cx.b.ins().umin(amt, c32);
            let k = cx.b.ins().iadd_imm(m32, -1);
            let cc = mutant(816).then_some(IntCC::UnsignedLessThan);
            (v, want.then(|| bit_dyn(cx, rm, k)), cc)
        }
        _ => {
            let v = cx.b.ins().rotr(rm, amt);
            (v, want.then(|| bit(cx, v, 31)), None)
        }
    };
    let carry = raw.map(|raw| {
        let found = match range {
            Some(cc) => {
                let inside = cx.b.ins().icmp_imm(cc, amt, 32);
                cx.b.ins().select(inside, raw, zero)
            }
            None => raw,
        };
        // a rotate by a non-zero multiple of 32 is the identity whose carry is bit 31
        let found = if ty == 3 && mutant(817) {
            let low = cx.b.ins().band_imm(amt, 31);
            let whole = cx.b.ins().icmp_imm(IntCC::Equal, low, 0);
            let old = cx.flag(1);
            cx.b.ins().select(whole, old, found)
        } else {
            found
        };
        let none = cx.b.ins().icmp_imm(IntCC::Equal, amt, 0);
        let old = cx.flag(1);
        cx.b.ins().select(none, old, found)
    });
    Operand { value, carry }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::native::emit::testing::build_and_verify;
    use crate::machine::sb::uop::k;

    #[test]
    fn every_form_type_and_amount_builds_verified_ir_and_only_lsl_0_keeps_c() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        build_and_verify(&sb, |cx| {
            for ty in 0..4u8 {
                for amount in [0u8, 1, 15, 31] {
                    let mut u = Uop::new(k::dp(1, 13, true), 0x1000);
                    (u.rd, u.rm, u.sh) = (0, 1, ty << 5 | amount);
                    let o = operand(cx, &u, 1, true);
                    assert_eq!(o.carry.is_none(), (ty, amount) == (0, 0), "{ty} {amount}");
                    assert!(operand(cx, &u, 1, false).carry.is_none(), "not asked for");
                }
                let mut u = Uop::new(k::dp(2, 13, true), 0x1000);
                (u.rd, u.rm, u.rs, u.sh) = (0, 1, 2, ty << 5);
                assert!(operand(cx, &u, 2, true).carry.is_some());
                assert!(operand(cx, &u, 2, false).carry.is_none());
            }
            let mut u = Uop::new(k::dp(0, 13, true), 0x1000);
            u.imm = 0x8000_0000;
            assert!(operand(cx, &u, 0, true).carry.is_some());
            u.sh = 1;
            assert!(
                operand(cx, &u, 0, true).carry.is_none(),
                "rotation 0 keeps C"
            );
            cx.exit_generic(0x1000);
        });
    }
}
