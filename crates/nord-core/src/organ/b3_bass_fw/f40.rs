//! 40-bit extended float and fixed multiplier of the SHARC, bit exact.
//!
//! VERIFIED: the rules are `tools/sharc_listing_vm.py` / `tools/sharc_listing_ext.py`, which match
//! the per instruction trace of `sharc-chain` (`docs/research/organ-farfisa-bass.md` s13 / s14); the
//! tests below check the Rust port against vectors produced by those Python functions.
//!
//! A value is `sign(1) | exponent(8) | fraction(31)` in bits 39..0 plus the implicit leading one,
//! i.e. a 32 bit significand: `value32 << 8 | extension`.

use super::fast;

pub(super) const M32: u64 = 0xFFFF_FFFF;
const ALL_ONES: u64 = 0xFF_FFFF_FFFF;

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Zero,
    Inf,
    Nan,
    Norm,
}

fn unpack(v: u64) -> (bool, Class, i32, u128) {
    let neg = v >> 39 == 1;
    let e = ((v >> 31) & 0xFF) as i32;
    let frac = v & 0x7FFF_FFFF;
    let cls = match e {
        0 => Class::Zero,
        255 if frac == 0 => Class::Inf,
        255 => Class::Nan,
        _ => Class::Norm,
    };
    let man = if cls == Class::Norm {
        u128::from((1u64 << 31) | frac)
    } else {
        0
    };
    (neg, cls, e - 127, man)
}

/// Rounds `m * 2^e2` to a 32 bit significand (nearest even) and packs the 40 bit float.
fn round_pack(neg: bool, e2: i32, m: u128, sticky: bool) -> u64 {
    let sign = u64::from(neg) << 39;
    if m == 0 {
        return sign;
    }
    let msb = 127 - m.leading_zeros() as i32;
    let mut exp = e2 + msb;
    let shift = msb + 1 - 32;
    let mut q = if shift > 0 {
        let rem = m & ((1u128 << shift) - 1);
        let half = 1u128 << (shift - 1);
        let q = m >> shift;
        if rem > half || (rem == half && (sticky || q & 1 == 1)) {
            q + 1
        } else {
            q
        }
    } else {
        m << -shift
    };
    if q == 1 << 32 {
        q >>= 1;
        exp += 1;
    }
    if exp > 127 {
        return sign | (0xFF << 31);
    }
    if exp < -126 {
        return sign;
    }
    sign | (((exp + 127) as u64) << 31) | (q as u64 & 0x7FFF_FFFF)
}

/// `Fx + Fy` (`sub` = `Fx - Fy`): the 64 bit fast path for normal operands, else the general one.
#[inline(always)]
pub(super) fn add(a: u64, b: u64, sub: bool) -> u64 {
    fast::add(a, b, sub).unwrap_or_else(|| add_general(a, b, sub))
}

/// `Fx * Fy`: the 64 bit fast path for normal operands, else the general one.
#[inline(always)]
pub(super) fn mul(a: u64, b: u64) -> u64 {
    fast::mul(a, b).unwrap_or_else(|| mul_general(a, b))
}

/// `Fx + Fy` on any operands (128 bit integers).
pub(super) fn add_general(a: u64, b: u64, sub: bool) -> u64 {
    let (mut xn, xc, mut xe, mut xm) = unpack(a);
    let (mut yn, yc, mut ye, mut ym) = unpack(b);
    yn ^= sub;
    if xc == Class::Nan || yc == Class::Nan || (xc == Class::Inf && yc == Class::Inf && xn != yn) {
        return ALL_ONES;
    }
    if xc == Class::Inf {
        return (u64::from(xn) << 39) | (0xFF << 31);
    }
    if yc == Class::Inf {
        return (u64::from(yn) << 39) | (0xFF << 31);
    }
    if xc == Class::Zero && yc == Class::Zero {
        return u64::from(xn && yn) << 39;
    }
    if xc == Class::Zero {
        return round_pack(yn, ye - 31, ym, false);
    }
    if yc == Class::Zero {
        return round_pack(xn, xe - 31, xm, false);
    }
    if (xe, xm) < (ye, ym) {
        (xn, xe, xm, yn, ye, ym) = (yn, ye, ym, xn, xe, xm);
    }
    let d = (xe - ye) as u32;
    let (mb, full) = (xm << 64, ym << 64);
    let (ms, sticky) = if d >= 128 {
        (0, true)
    } else {
        (full >> d, full & ((1u128 << d) - 1) != 0)
    };
    let m = if xn == yn {
        mb + ms
    } else {
        mb - ms - u128::from(sticky)
    };
    if m == 0 && !sticky {
        return 0;
    }
    round_pack(xn, xe - 95, m, sticky)
}

/// `Fx * Fy` on any operands.
pub(super) fn mul_general(a: u64, b: u64) -> u64 {
    let (xn, xc, xe, xm) = unpack(a);
    let (yn, yc, ye, ym) = unpack(b);
    let neg = xn ^ yn;
    let has = |c: Class| xc == c || yc == c;
    if has(Class::Nan) || (has(Class::Inf) && has(Class::Zero)) {
        return 0x7F_FFFF_FFFF | (u64::from(neg) << 39);
    }
    if has(Class::Inf) {
        return (u64::from(neg) << 39) | (0xFF << 31);
    }
    if has(Class::Zero) {
        return u64::from(neg) << 39;
    }
    round_pack(neg, xe + ye - 62, xm * ym, false) // 32 x 32 bit product fits u64 (checked by the oracle test)
}

/// `PASS Fx` (a denormal reads as zero).
#[inline(always)]
pub(super) fn pass(a: u64) -> u64 {
    fast::pass(a)
}

/// `PASS Fx` on the 128 bit unpacked form (reference for [`pass`]).
#[cfg(test)]
pub(super) fn pass_general(a: u64) -> u64 {
    let (neg, cls, e, man) = unpack(a);
    match cls {
        Class::Norm => {
            (u64::from(neg) << 39) | (((e + 127) as u64) << 31) | (man as u64 & 0x7FFF_FFFF)
        }
        Class::Zero => u64::from(neg) << 39,
        _ => ALL_ONES,
    }
}

/// `float Rx by Ry`: the integer `x` scaled by `2^scale`.
#[inline(always)]
pub(super) fn float_by(x: i32, scale: i32) -> u64 {
    if x == 0 {
        0
    } else {
        fast::float_by(x, scale)
    }
}

/// [`float_by`] on 128 bit integers (reference).
#[cfg(test)]
pub(super) fn float_by_general(x: i32, scale: i32) -> u64 {
    if x == 0 {
        0
    } else {
        round_pack(x < 0, scale, u128::from(x.unsigned_abs()), false)
    }
}

/// `abs Fx`.
pub(super) fn abs(f: u64) -> u64 {
    if (f >> 31) & 0xFF != 0 {
        f & !(1 << 39)
    } else {
        0
    }
}

/// Left shift by `s` (right when negative); off scale gives 0 or the sign fill.
pub(super) fn shift(v: u32, s: i32, arith: bool) -> u32 {
    if s >= 0 {
        return if s < 32 { v << s } else { 0 };
    }
    if -s >= 32 {
        return if arith && v >> 31 == 1 { u32::MAX } else { 0 };
    }
    if arith {
        ((v as i32) >> -s) as u32
    } else {
        v >> -s
    }
}

fn low_mask(n: u32) -> u32 {
    if n >= 32 {
        u32::MAX
    } else {
        (1u32 << n) - 1
    }
}

/// `fdep`: deposits the low `length` bits of `x` at `pos`.
pub(super) fn fdep(x: u32, length: u32, pos: u32, se: bool) -> u32 {
    let length = length.min(32);
    let field = x & low_mask(length);
    let wide = if pos < 32 { field << pos } else { 0 };
    let top = pos + length;
    let mut out = wide;
    if se && length > 0 && top <= 32 && (field >> (length - 1)) & 1 == 1 {
        out |= if top >= 32 { 0 } else { u32::MAX << top };
    }
    out
}

/// `fext`: extracts `length` bits of `x` from `pos`.
pub(super) fn fext(x: u32, length: u32, pos: u32, se: bool) -> u32 {
    let length = length.min(32);
    let mut v = (if pos < 32 { x >> pos } else { 0 }) & low_mask(length);
    if se && length > 0 && pos + length <= 32 && (v >> (length - 1)) & 1 == 1 {
        v |= !low_mask(length);
    }
    v
}

fn wrap80(v: i128) -> i128 {
    let v = v & ((1i128 << 80) - 1);
    if v >> 79 != 0 {
        v - (1i128 << 80)
    } else {
        v
    }
}

fn round_mr(v: i128) -> i128 {
    let mut r = v + 0x8000_0000;
    if v & i128::from(M32 as u32) == 0x8000_0000 {
        r &= !(1i128 << 32);
    }
    wrap80(r & !i128::from(M32 as u32))
}

/// Fixed point multiply of the register words `x` and `y` (`mode` bits: 1 signed x, 2 signed y,
/// 4 fractional, 8 round), accumulating on `acc` (`mr_op` 0 none, 1 `+`, 2 `-`). Returns the new
/// 80 bit accumulator value (`MRF=` form) and the 32 bit result word (`Rn=` form).
pub(super) fn mult(x: u32, y: u32, acc: i128, mr_op: i32, mode: i32) -> (i128, u32) {
    let (sx, sy, frac) = (mode & 1 != 0, mode & 2 != 0, mode & 4 != 0);
    let op = |v: u32, signed: bool| {
        if signed {
            i128::from(v as i32)
        } else {
            i128::from(v)
        }
    };
    let mut p = op(x, sx) * op(y, sy);
    if frac && sx && sy {
        p <<= 1;
    }
    let mut v = wrap80(match mr_op {
        0 => p,
        1 => acc + p,
        _ => acc - p,
    });
    if frac && mode & 8 != 0 {
        v = round_mr(v);
    }
    let word = if frac { (v >> 32) as u32 } else { v as u32 };
    (v, word)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vectors from `tools/sharc_listing_vm.py` / `sharc_listing_ext.py` (run 2026-11-14).
    fn f(x: f32) -> u64 {
        u64::from(x.to_bits()) << 8
    }

    #[test]
    fn simple_float_arithmetic_matches_f32_on_exact_cases() {
        assert_eq!(add(f(1.0), f(2.0), false), f(3.0));
        assert_eq!(add(f(1.0), f(2.0), true), f(-1.0));
        assert_eq!(mul(f(1.5), f(-4.0)), f(-6.0));
        assert_eq!(pass(f(0.25)), f(0.25));
        assert_eq!(abs(f(-0.5)), f(0.5));
        assert_eq!(add(f(1.0), f(-1.0), false), 0);
    }

    #[test]
    fn the_significand_has_32_bits_not_24() {
        // 1 + 2^-25 needs 26 bits: single precision rounds it away, the 40 bit float keeps it
        // (bit 6 of the 31 bit fraction); 1 + 2^-40 is below the last bit and rounds back to 1.
        let tiny = f(f32::powi(2.0, -25));
        assert_eq!(add(f(1.0), tiny, false), f(1.0) | 0x40);
        assert_eq!(add(f(1.0), f(f32::powi(2.0, -40)), false), f(1.0));
        assert_eq!(pass(f(1.0) | 0x55), f(1.0) | 0x55);
    }

    #[test]
    fn float_by_scales_integers() {
        assert_eq!(float_by(3, 0), f(3.0));
        assert_eq!(float_by(-5, -2), f(-1.25));
        assert_eq!(float_by(0, 7), 0);
    }

    #[test]
    fn field_ops_follow_the_python_rules() {
        assert_eq!(fdep(0xFFFF, 7, 25, false), 0xFE00_0000);
        assert_eq!(fext(0xFE00_0000, 7, 25, false), 0x7F);
        assert_eq!(fext(0xFE00_0000, 7, 25, true), 0xFFFF_FFFF);
        assert_eq!(shift(0x8000_0001, 1, false), 2);
        assert_eq!(shift(0x8000_0000, -31, true), 0xFFFF_FFFF);
        assert_eq!(shift(5, 40, true), 0);
    }

    #[test]
    fn fixed_multiplier_rounds_and_accumulates() {
        // 0.5 * 0.5 in 1.31 fractional: 0.25 = 0x2000_0000
        let (acc, word) = mult(0x4000_0000, 0x4000_0000, 0, 0, 7);
        assert_eq!(word, 0x2000_0000);
        let (acc2, word2) = mult(0x4000_0000, 0x4000_0000, acc, 1, 7);
        assert_eq!(word2, 0x4000_0000);
        assert!(acc2 > acc);
        // round to nearest even on the low 32 bits
        assert_eq!(round_mr(0x8000_0000), 0);
        assert_eq!(round_mr(0x1_8000_0000), 0x2_0000_0000);
        assert_eq!(wrap80(1i128 << 79), -(1i128 << 79));
    }
}
