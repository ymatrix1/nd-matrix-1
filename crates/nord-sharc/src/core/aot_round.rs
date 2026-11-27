//! Shared helpers of the native float kernels in `aot_fast.rs`: operand classification, the
//! zero/denormal paths, and the single rounding step.
//!
//! The rounding takes an already normalized significand (most significant bit at bit 63), so the
//! slow "shift up when the value is exact, shift down when it needs rounding" split of
//! `float::round_pack` disappears: a value that needs no rounding has zeros in the discarded low
//! bits, and the same shift and add handle it. Round-to-nearest-even is one add and one shift:
//! with `q` the kept bits, `rem` the discarded bits, `half = 2^(sh-1)` and `t = sticky | (q & 1)`,
//! `rem > half || (rem == half && t)` equals `(rem + half - 1 + t) >> sh == 1`.
//!
//! Provenance: ASSUMED equal to `float::round_pack` by construction; VERIFIED bit-identical by the
//! equivalence sweeps in `aot_fast.rs` and `aot_fast_sweep.rs`, which compare every `Out` field.
use super::float::{self, overflow_value, Mode, Out};

/// Exponent field value of infinity and NaN.
const EXP_SPECIAL: u64 = 255;

/// Exponent field (bits 38..31) of a 40-bit float.
#[inline(always)]
pub(super) fn exp_field(v: u64) -> u64 {
    (v >> 31) & 0xFF
}

/// Whether both operands are normal numbers (exponent field 1..=254): not zero or denormal, not
/// infinity or NaN. One flag for the pair, so the fast path has a single predictable branch.
#[inline(always)]
pub(super) fn both_normal(a: u64, b: u64) -> bool {
    (exp_field(a).wrapping_sub(1) < 254) & (exp_field(b).wrapping_sub(1) < 254)
}

/// Rounds the normalized significand `mn` (bit 63 set) with unbiased exponent `exp` of bit 63, plus
/// a non-zero fraction of the last place when `sticky`, to a 24-bit (`narrow`) or 32-bit
/// significand and encodes it. Overflow and flush take [`exp_exception`].
///
/// Round to nearest even is an add on the whole significand: with `sh` the number of discarded
/// bits (32, or 40 when narrow), adding `2^(sh-1) - 1 + t` (`t` = sticky or last kept bit) carries
/// into the kept bits exactly when the discarded part is above one half, or equal to it and `t`.
/// A carry out of bit 63 means an all-ones significand rounded up: the fraction is then zero and
/// the exponent grows by one. The fraction field is the masked top half of the sum, so no
/// width-dependent variable shift remains (a variable shift is 3 micro-ops on x86 without BMI2).
#[inline(always)]
pub(super) fn round_norm(
    neg: bool,
    exp: i64,
    mn: u64,
    sticky: bool,
    mode: Mode,
    narrow: bool,
) -> Out {
    debug_assert!(mn >> 63 == 1);
    // (round position, 2^(sh-1) - 1, fraction bits of `sum >> 32` in the register layout)
    let (sh, half_m1, fmask) = if narrow {
        (40, (1u64 << 39) - 1, 0x7FFF_FF00)
    } else {
        (32, (1u64 << 31) - 1, 0x7FFF_FFFF)
    };
    let t = u64::from(sticky) | ((mn >> sh) & 1);
    let add = (half_m1 + t) & u64::from(!mode.trunc).wrapping_neg();
    let (sum, carry) = mn.overflowing_add(add);
    let exp = exp + i64::from(carry);
    if (exp + 126) as u64 > 253 {
        return unpack(exp_exception_p(neg, exp, mode, narrow));
    }
    let v = (u64::from(neg) << 39) | (((exp + 127) as u64) << 31) | ((sum >> 32) & fmask);
    // a normal result: biased exponent 1..=254, so AZ is clear and AN is the sign
    Out {
        v,
        an: neg,
        ..Out::default()
    }
}

/// An [`Out`] in two registers: a cold function returning an `Out` writes it to memory, and the
/// compiler then keeps the hot path's `Out` in the same memory slot (stores, then reloads of every
/// field by the caller). Returning this pair keeps both paths in registers.
pub(super) type Packed = (u64, u64);

/// `o` as a [`Packed`] pair.
#[inline(always)]
pub(super) fn pack(o: Out) -> Packed {
    let flags = u64::from(o.az)
        | u64::from(o.an) << 1
        | u64::from(o.av) << 2
        | u64::from(o.as_) << 3
        | u64::from(o.ai) << 4
        | u64::from(o.mn) << 5
        | u64::from(o.mv) << 6
        | u64::from(o.mu) << 7
        | u64::from(o.mi) << 8
        | u64::from(o.au) << 9;
    (o.v, flags)
}

/// The inverse of [`pack`].
#[inline(always)]
pub(super) fn unpack((v, f): Packed) -> Out {
    let bit = |n: u32| (f >> n) & 1 == 1;
    Out {
        v,
        az: bit(0),
        an: bit(1),
        av: bit(2),
        as_: bit(3),
        ai: bit(4),
        mn: bit(5),
        mv: bit(6),
        mu: bit(7),
        mi: bit(8),
        au: bit(9),
    }
}

/// The result when the rounded exponent is above 127 (overflow) or below -126 (flushed to zero).
#[cold]
#[inline(never)]
fn exp_exception_p(neg: bool, exp: i64, mode: Mode, narrow: bool) -> Packed {
    pack(exp_exception(neg, exp, mode, narrow))
}

fn exp_exception(neg: bool, exp: i64, mode: Mode, narrow: bool) -> Out {
    if exp > 127 {
        return Out::value(overflow_value(neg, mode, narrow), true, false);
    }
    Out {
        au: true,
        ..Out::value(u64::from(neg) << 39, false, true)
    }
}

/// `Fx +/- Fy` when an operand is zero or denormal (flushed to zero): the other operand passes
/// through exactly (RND32 off) and a zero sum is `+0`; anything else (infinity, NaN, RND32)
/// delegates to the reference kernel.
#[inline(always)]
pub(super) fn add_with_zero(a: u64, b: u64, sub: bool, mode: Mode) -> Out {
    unpack(add_with_zero_p(a, b, sub, mode))
}

#[cold]
#[inline(never)]
fn add_with_zero_p(a: u64, b: u64, sub: bool, mode: Mode) -> Packed {
    pack(add_with_zero_out(a, b, sub, mode))
}

fn add_with_zero_out(a: u64, b: u64, sub: bool, mode: Mode) -> Out {
    let (fa, fb) = (exp_field(a), exp_field(b));
    if mode.r32 || fa == EXP_SPECIAL || fb == EXP_SPECIAL {
        return float::add(a, b, sub, false, mode);
    }
    let nb = (b >> 39 == 1) ^ sub;
    match (fa == 0, fb == 0) {
        (true, true) => Out {
            v: u64::from(a >> 39 == 1 && nb) << 39,
            az: true,
            an: a >> 39 == 1 && nb,
            ..Out::default()
        },
        (true, false) => {
            let v = (b & ((1 << 39) - 1)) | u64::from(nb) << 39;
            Out {
                v,
                an: nb,
                ..Out::default()
            }
        }
        _ => Out {
            v: a,
            an: a >> 39 == 1,
            ..Out::default()
        },
    }
}

/// `Fx * Fy` when an operand is zero or denormal: a zero of the product's sign with MN set when it
/// is negative, unless the other operand is infinity or NaN (reference kernel).
#[inline(always)]
pub(super) fn mul_with_zero(a: u64, b: u64, mode: Mode) -> Out {
    unpack(mul_with_zero_p(a, b, mode))
}

#[cold]
#[inline(never)]
fn mul_with_zero_p(a: u64, b: u64, mode: Mode) -> Packed {
    pack(mul_with_zero_out(a, b, mode))
}

fn mul_with_zero_out(a: u64, b: u64, mode: Mode) -> Out {
    let (fa, fb) = (exp_field(a), exp_field(b));
    if fa == EXP_SPECIAL || fb == EXP_SPECIAL {
        return float::mul(a, b, mode);
    }
    let neg = (a >> 39) != (b >> 39);
    Out {
        v: u64::from(neg) << 39,
        mn: neg,
        ..Out::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_and_unpack_are_inverses_for_every_flag() {
        for bits in 0..1024u64 {
            let o = unpack((0xAB_CDEF_0123, bits));
            assert_eq!(pack(o), (0xAB_CDEF_0123, bits));
        }
    }

    #[test]
    fn normal_classification_matches_the_exponent_ranges() {
        let f = |e: u64| e << 31;
        assert!(both_normal(f(1), f(254)));
        assert!(!both_normal(f(0), f(100)));
        assert!(!both_normal(f(100), f(255)));
        assert!(!both_normal(f(255), f(0)));
        assert_eq!(exp_field(0xFF_FFFF_FFFF), 255);
    }

    #[test]
    fn round_norm_rounds_ties_to_even_and_carries_out_of_all_ones() {
        let m = Mode::default();
        // significand 1.0 exactly: no rounding
        let one = round_norm(false, 0, 1 << 63, false, m, false);
        assert_eq!(one.v, 127 << 31);
        // tie, even below: stays; tie, odd below: rounds up; sticky breaks the tie upward
        let tie = (1u64 << 63) | (1 << 31);
        assert_eq!(round_norm(false, 0, tie, false, m, false).v, 127 << 31);
        assert_eq!(
            round_norm(false, 0, tie | (1 << 32), false, m, false).v,
            (127 << 31) | 2
        );
        assert_eq!(round_norm(false, 0, tie, true, m, false).v, (127 << 31) | 1);
        // all ones plus a half rounds to the next power of two
        let ones = u64::MAX;
        assert_eq!(round_norm(false, 0, ones, false, m, false).v, 128 << 31);
        // truncation drops the discarded bits
        let tr = Mode { trunc: true, ..m };
        assert_eq!(
            round_norm(false, 0, ones, false, tr, false).v,
            (127 << 31) | 0x7FFF_FFFF
        );
    }

    #[test]
    fn round_norm_flushes_and_overflows_through_the_cold_path() {
        let m = Mode::default();
        let hi = round_norm(false, 128, 1 << 63, false, m, false);
        assert!(hi.av && hi.v == 0xFF << 31);
        let lo = round_norm(true, -127, 1 << 63, false, m, false);
        assert!(lo.au && lo.az && lo.an && lo.v == 1 << 39);
        // rounding carry pushes the largest value over
        let top = round_norm(false, 127, u64::MAX, false, m, false);
        assert!(top.av);
    }
}
