//! OWNER: W2-NDP. Add and subtract with carry and the four flags, from SSA operands (the native
//! image of `alu::add_c` and the arithmetic rows of `exec_alu::dp`).
//!
//! `x + y + cin` with `cin` 0, 1 or the carry flag. The result is a plain wrapping add (a plain
//! `isub` when the form is `a - b`). With `flags` the four flags are written:
//!
//! | Flag | Definition |
//! |---|---|
//! | N | bit 31 of the result |
//! | Z | result is 0 |
//! | C | carry out of bit 31: `res < x` (cin 0), `res <= x` (cin 1), bit 32 of the 64-bit sum (cin the flag) |
//! | V | `(!(x ^ y) & (x ^ res)) >> 31`, the signed overflow of the addition |
//!
//! Subtraction is `a + !b + 1` (carry is "no borrow"); `adc`, `sbc` and `rsc` use the carry
//! flag as it was before the instruction (read once, before any flag is written).
//!
//! Provenance: VERIFIED against `alu::add_c` (through the tier-2 `exec_alu::exec_dp`) by the
//! differential tests of `lower_dp/diff.rs` (every arithmetic opcode on 13 x 13 edge operand
//! pairs x 16 flag nibbles, and random states per id); mutants 800..=802, 808, 814, 821 and
//! 827 of `lower_dp/kill.rs` are killed by them.

use super::{b2i, mutant, set_nz};
use crate::machine::sb::native::emit::NCtx;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};

/// The carry into the addition.
#[derive(Clone, Copy)]
pub(super) enum Cin {
    Zero,
    One,
    /// An `i32` 0 or 1 (the carry flag, possibly inverted by a mutant).
    Val(Value),
}

/// `x + y + cin`; with `flags` also N, Z, C and V. `vy` is the addend the overflow rule sees
/// (`y`; a mutant passes another one).
pub(super) fn add3(
    cx: &mut NCtx<'_>,
    x: Value,
    y: Value,
    vy: Value,
    cin: Cin,
    flags: bool,
) -> Value {
    let sum = cx.b.ins().iadd(x, y);
    let res = match cin {
        Cin::Zero => sum,
        Cin::One => cx.b.ins().iadd_imm(sum, 1),
        Cin::Val(c) => cx.b.ins().iadd(sum, c),
    };
    if flags {
        set_nz(cx, res);
        let carry = match cin {
            Cin::Zero => {
                let lt = cx.b.ins().icmp(IntCC::UnsignedLessThan, res, x);
                b2i(cx, lt)
            }
            Cin::One => {
                let le = cx.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, res, x);
                b2i(cx, le)
            }
            Cin::Val(c) => {
                let wx = cx.b.ins().uextend(types::I64, x);
                let wy = cx.b.ins().uextend(types::I64, y);
                let wc = cx.b.ins().uextend(types::I64, c);
                let wc = if mutant(827) {
                    cx.b.ins().iconst(types::I64, 0)
                } else {
                    wc
                };
                let wide = cx.b.ins().iadd(wx, wy);
                let wide = cx.b.ins().iadd(wide, wc);
                let hi = cx.b.ins().ushr_imm(wide, 32);
                cx.b.ins().ireduce(types::I32, hi)
            }
        };
        cx.set_flag(1, carry);
        let diff = cx.b.ins().bxor(x, res);
        let same = cx.b.ins().bxor(x, vy);
        let ovf = cx.b.ins().band_not(diff, same);
        let ovf = cx.b.ins().ushr_imm(ovf, 31);
        cx.set_flag(0, ovf);
    }
    res
}

/// `p - q` as `p + !q + cin` (`cin` One for a plain subtract). A flag-less `a - b` is one
/// `isub`.
pub(super) fn sub3(cx: &mut NCtx<'_>, p: Value, q: Value, cin: Cin, flags: bool) -> Value {
    if !flags && matches!(cin, Cin::One) {
        return cx.b.ins().isub(p, q);
    }
    let nq = cx.b.ins().bnot(q);
    let vy = if mutant(814) { q } else { nq };
    add3(cx, p, nq, vy, cin, flags)
}

/// The arithmetic opcodes `2..=7`, `10` and `11` on `a` and `b`; the result is returned and
/// the flags written when `flags`.
pub(super) fn arith(cx: &mut NCtx<'_>, op: u32, a: Value, b: Value, flags: bool) -> Value {
    let c = cx.flag(1);
    let res = match op {
        2 | 10 if mutant(821) => add3(cx, a, b, b, Cin::One, flags),
        2 | 10 => sub3(cx, a, b, Cin::One, flags),
        3 if mutant(802) => sub3(cx, a, b, Cin::One, flags),
        3 => sub3(cx, b, a, Cin::One, flags),
        4 | 11 => add3(cx, a, b, b, Cin::Zero, flags),
        5 if mutant(800) => add3(cx, a, b, b, Cin::Zero, flags),
        5 => add3(cx, a, b, b, Cin::Val(c), flags),
        6 => {
            let c = if mutant(801) {
                cx.b.ins().bxor_imm(c, 1)
            } else {
                c
            };
            sub3(cx, a, b, Cin::Val(c), flags)
        }
        _ => sub3(cx, b, a, Cin::Val(c), flags),
    };
    if flags && op == 10 && mutant(808) {
        let c = cx.flag(1);
        let inv = cx.b.ins().bxor_imm(c, 1);
        cx.set_flag(1, inv);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::native::emit::testing::build_and_verify;

    #[test]
    fn every_carry_form_builds_verified_ir_and_flags_are_written_only_on_request() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        build_and_verify(&sb, |cx| {
            let (a, b) = (cx.reg(0), cx.reg(1));
            for op in [2, 3, 4, 5, 6, 7, 10, 11] {
                cx.flags_def = 0;
                let _ = arith(cx, op, a, b, false);
                assert_eq!(cx.flags_def, 0, "op {op} without S writes no flag");
                let _ = arith(cx, op, a, b, true);
                assert_eq!(cx.flags_def, 0b1111, "op {op} with S writes N Z C V");
            }
            cx.exit_generic(0x1000);
        });
    }
}
