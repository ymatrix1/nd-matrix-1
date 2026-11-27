//! Native-width float kernels for the hot-block AOT prototype: `Fx + Fy`, `Fx - Fy`, `Fx * Fy`
//! and `FLOAT Rx BY Ry` on 40-bit registers, equal to `float::add`, `float::mul` and
//! `float_conv::from_int` result for result and flag for flag, but with `u64` arithmetic instead
//! of the `u128` of the reference kernels and a direct path for ordinary (normal, non-zero)
//! operands (add: `aot_fadd.rs`, rounding: `aot_round.rs`). Zero and denormal operands, and
//! results that overflow or flush, are handled by cold paths in `aot_round.rs` (a zero operand
//! passes the other through, a zero product is a signed zero); infinity, NaN and RND32 with a
//! zero operand delegate to the reference kernel, so the remaining special-value rules stay in
//! one place.
//!
//! Provenance: ASSUMED equal to the reference kernels by construction; the claim "bit-identical"
//! is VERIFIED by the randomized equivalence tests below (`fast_*_equals_reference_*`: operand
//! classes x RND32 x TRUNC, every `Out` field compared) and by the real-firmware differential in
//! `docs/research/sharc-aot-hot-block.md`. The reference kernels themselves are VERIFIED against
//! the CCES simulator (see `float.rs`).
use super::aot_fadd::add_normal;
use super::aot_round::{add_with_zero, both_normal, exp_field, mul_with_zero, round_norm};
use super::decode_cache::ComputeOp;
#[cfg(test)]
use super::float;
use super::float::{Mode, Out};
use super::float_conv;
use super::Core;

/// Mask that clears the 8 extension bits when RND32 is set, and nothing otherwise.
#[inline(always)]
fn operand_mask(mode: Mode) -> u64 {
    !(0xFFu64 * u64::from(mode.r32))
}

/// The 32-bit significand of a normal float with its implicit leading one.
#[inline(always)]
pub(super) fn significand(v: u64) -> u64 {
    (1 << 31) | (v & 0x7FFF_FFFF)
}

/// `Fx + Fy` (`sub` = false) or `Fx - Fy` on 40-bit floats, like `float::add(a, b, sub, false, m)`.
///
/// Normal operands go through the double-precision path of `aot_fadd.rs` (one hardware add, one
/// rounding); zero, denormal, infinite and NaN operands take the cold path, and the rare cases
/// the double path cannot decide (see `aot_fadd.rs`) take the exact integer kernel.
#[inline(always)]
pub(super) fn fast_add(a: u64, b: u64, sub: bool, mode: Mode) -> Out {
    let keep = operand_mask(mode);
    let (a, b) = (a & keep, b & keep);
    if !both_normal(a, b) {
        return add_with_zero(a, b, sub, mode);
    }
    add_normal(a, b ^ (u64::from(sub) << 39), mode)
}

/// `Fx * Fy` on 40-bit floats, like `float::mul(a, b, m)`.
///
/// The 32x32 significand product has its top bit at 63 or 62, so the normalization is a shift by
/// 0 or 1 and the rounding happens once in [`round_norm`].
#[inline(always)]
pub(super) fn fast_mul(a: u64, b: u64, mode: Mode) -> Out {
    let keep = operand_mask(mode);
    let (a, b) = (a & keep, b & keep);
    if !both_normal(a, b) {
        return mul_with_zero(a, b, mode);
    }
    let neg = (a ^ b) >> 39 != 0;
    let p = significand(a) * significand(b);
    // top bit at 63 or 62: shift left by one (p + p) in the second case
    let low = u64::from(p >> 63 == 0);
    let exp = (exp_field(a) + exp_field(b)) as i64 - 253 - low as i64;
    let o = round_norm(
        neg,
        exp,
        p + (p & low.wrapping_neg()),
        false,
        mode,
        mode.r32,
    );
    Out {
        mn: neg,
        mv: o.av,
        mu: o.az,
        ..o
    }
}

/// `Fn = FLOAT Rx BY scale` (fixed to float), like `float_conv::from_int(x, scale, m)`.
#[inline(always)]
pub(super) fn fast_from_int(x: i32, scale: i32, mode: Mode) -> Out {
    if x == 0 {
        return float_conv::from_int(x, scale, mode);
    }
    let m = u64::from(x.unsigned_abs());
    let lz = m.leading_zeros();
    // bit 63 of `m << lz` has the weight 2^(scale + 63 - lz)
    let exp = i64::from(scale) + 63 - i64::from(lz);
    round_norm(x < 0, exp, m << lz, false, mode, false)
}

/// ASTAT bit 10: the last ALU operation was floating-point (set by every float ALU op).
const ASTAT_AF: u32 = 1 << 10;

// Used only by generated block modules (see `aot_hot.rs`) and the tests below.
#[allow(dead_code)]
impl Core {
    /// `Fn = Fx + Fy` / `Fx - Fy` through [`fast_add`], effects identical to `float_alu`.
    #[inline(always)]
    pub(super) fn aot_fadd(&mut self, n: u8, x: u8, y: u8, sub: bool) {
        let mode = Mode::from_mode1(self.mode1);
        let o = fast_add(
            self.f40(usize::from(x)),
            self.f40(usize::from(y)),
            sub,
            mode,
        );
        self.float_result(usize::from(n), o);
        self.sys.astat_extra[0] |= ASTAT_AF;
    }

    /// `Fn = FLOAT Rx BY Ry` through [`fast_from_int`].
    #[inline(always)]
    pub(super) fn aot_float(&mut self, n: u8, x: u8, y: u8) {
        let mode = Mode::from_mode1(self.mode1);
        let (ix, iy) = (self.r[usize::from(x)] as i32, self.r[usize::from(y)] as i32);
        let o = fast_from_int(ix, iy, mode);
        self.float_result(usize::from(n), o);
        self.sys.astat_extra[0] |= ASTAT_AF;
    }

    /// `Fn = Fx * Fy` through [`fast_mul`], effects identical to `float_mul`.
    #[inline(always)]
    pub(super) fn aot_fmul(&mut self, n: u8, x: u8, y: u8) {
        let mode = Mode::from_mode1(self.mode1);
        let o = fast_mul(self.f40(usize::from(x)), self.f40(usize::from(y)), mode);
        self.mul_result(usize::from(n), o);
    }

    /// [`Core::aot_op`] with the native-width float kernels for add, subtract, `FLOAT` and
    /// multiply; every other compute field runs the interpreter's implementation.
    #[inline(always)]
    pub(super) fn aot_op_native(&mut self, op: ComputeOp) {
        match op {
            ComputeOp::FloatAlu { op: 0x81, n, x, y } => self.aot_fadd(n, x, y, false),
            ComputeOp::FloatAlu { op: 0x82, n, x, y } => self.aot_fadd(n, x, y, true),
            ComputeOp::FloatAlu { op: 0xDA, n, x, y } => self.aot_float(n, x, y),
            ComputeOp::FloatMul { n, x, y } => self.aot_fmul(n, x, y),
            other => self.aot_op(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift stream.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// A 40-bit float drawn from an operand class: ordinary, near-1, huge/tiny exponents, exact
    /// powers of two, specials (zero, denormal, infinity, NaN) and values with a set extension.
    fn operand(r: &mut Rng) -> u64 {
        let v = r.next();
        let (sign, frac, ext) = ((v >> 63) << 39, (v >> 8) & 0x7FFF_FFFF, v & 0xFF);
        let exp = match (v >> 40) % 16 {
            0 => 0,
            1 => 255,
            2 => 1,
            3 => 254,
            4 | 5 => 127 + r.next() % 5 - 2,
            6 => 100 + r.next() % 55,
            _ => 1 + r.next() % 254,
        };
        let frac = if (v >> 44).is_multiple_of(8) { 0 } else { frac };
        sign | exp << 31 | frac | ext
    }

    fn modes() -> [Mode; 4] {
        [(false, false), (true, false), (false, true), (true, true)].map(|(trunc, r32)| Mode {
            trunc,
            r32,
            sat: false,
        })
    }

    #[test]
    fn fast_add_and_sub_equal_reference_on_random_and_special_operands() {
        let mut r = Rng(0x1234_5678_9ABC_DEF1);
        for mode in modes() {
            for _ in 0..400_000 {
                let (a, b) = (operand(&mut r), operand(&mut r));
                for sub in [false, true] {
                    assert_eq!(
                        fast_add(a, b, sub, mode),
                        float::add(a, b, sub, false, mode),
                        "a={a:#x} b={b:#x} sub={sub} {mode:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn fast_add_equals_reference_for_cancellation_and_exponent_gaps() {
        let mut r = Rng(0xDEAD_BEEF_0BAD_F00D);
        for mode in modes() {
            for gap in 0..70u64 {
                for _ in 0..4_000 {
                    let a = operand(&mut r) & !(0xFFu64 << 31) | (150 + gap.min(100)) << 31;
                    let b = operand(&mut r) & !(0xFFu64 << 31) | 150 << 31;
                    // near-equal mantissas make the subtraction cancel
                    let b = if gap < 3 {
                        b & !0x3FF | (a & 0x3FF) ^ (r.next() & 3)
                    } else {
                        b
                    };
                    for sub in [false, true] {
                        assert_eq!(
                            fast_add(a, b, sub, mode),
                            float::add(a, b, sub, false, mode),
                            "a={a:#x} b={b:#x} sub={sub} {mode:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn fast_mul_equals_reference_including_overflow_and_flush() {
        let mut r = Rng(0x0F0F_1234_ABCD_5555);
        for mode in modes() {
            for _ in 0..600_000 {
                let (a, b) = (operand(&mut r), operand(&mut r));
                assert_eq!(
                    fast_mul(a, b, mode),
                    float::mul(a, b, mode),
                    "a={a:#x} b={b:#x} {mode:?}"
                );
            }
        }
    }

    #[test]
    fn fast_from_int_equals_reference_for_all_scales_used() {
        let mut r = Rng(0x7777_0101_2222_9999);
        for mode in modes() {
            for _ in 0..300_000 {
                let v = r.next();
                let x = match v % 4 {
                    0 => (v >> 8) as i32,
                    1 => ((v >> 8) as i32) >> ((v >> 40) % 32),
                    2 => i32::MIN + (v >> 60) as i32,
                    _ => (v >> 8) as i32 % 5,
                };
                for scale in [
                    -31,
                    0,
                    1,
                    -127,
                    120,
                    -200,
                    200,
                    i32::MAX,
                    i32::MIN,
                    (v >> 50) as i32 % 300 - 150,
                ] {
                    assert_eq!(
                        fast_from_int(x, scale, mode),
                        float_conv::from_int(x, scale, mode),
                        "x={x} scale={scale} {mode:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn native_ops_have_the_same_state_effects_as_the_interpreter_ops() {
        let ops = [
            ComputeOp::FloatAlu {
                op: 0x81,
                n: 3,
                x: 1,
                y: 2,
            },
            ComputeOp::FloatAlu {
                op: 0x82,
                n: 4,
                x: 2,
                y: 1,
            },
            ComputeOp::FloatAlu {
                op: 0xDA,
                n: 5,
                x: 6,
                y: 7,
            },
            ComputeOp::FloatMul { n: 8, x: 1, y: 2 },
            ComputeOp::FloatMul { n: 9, x: 3, y: 4 },
        ];
        let mut r = Rng(0xABCD_EF01_2345_6789);
        for case in 0..3000 {
            let mut slow = Core::new(&[]);
            slow.mode1 = [0, 1 << 15, 1 << 16, 1 << 21][case % 4];
            for k in 0..16 {
                slow.r[k] = (operand(&mut r) >> 8) as u32;
                slow.s[k] = (operand(&mut r) >> 8) as u32;
            }
            slow.r[7] = ((r.next() % 70) as i32 - 35) as u32;
            let mut fast = Core::new(&[]);
            fast.mode1 = slow.mode1;
            fast.r = slow.r;
            fast.s = slow.s;
            for op in ops {
                slow.aot_op(op);
                fast.aot_op_native(op);
                assert_eq!(slow.state_mismatch(&fast), None, "case {case} {op:?}");
            }
        }
    }
}
