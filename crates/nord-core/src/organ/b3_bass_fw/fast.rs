//! 64 bit fast paths of the 40 bit float add and multiply (normal operands).
//!
//! The general routines in `f40.rs` work on 128 bit integers so that every class (zero, infinity,
//! NaN) and every alignment is exact. For two normal operands the same result needs only 64 bits:
//! the 32 bit significands leave 31 guard bits below the rounding position, and an alignment shift
//! of 63 or more leaves a pure sticky bit. VERIFIED by `fast_paths_equal_the_general_routines`
//! (random operands, cancellation and every exponent distance against the 128 bit routines).
//!
//! A zero operand (biased exponent 0, any fraction) is handled here too: about 40 percent of the
//! adds and 45 percent of the multiplies of the Bass routines have one (measured on the chord
//! machine), and the 128 bit general path is slow. VERIFIED equal to the general routines by
//! `zero_operands_equal_the_general_routines`.

const MAG: u64 = 0x7F_FFFF_FFFF;
const FRAC: u64 = 0x7FFF_FFFF;
const ALL40: u64 = 0xFF_FFFF_FFFF;

/// Biased exponent field.
#[inline(always)]
fn exp(v: u64) -> u64 {
    (v >> 31) & 0xFF
}

/// Rounds `m * 2^e2` (`m != 0`) to a 32 bit significand, nearest even, and packs the 40 bit float.
///
/// `sticky` means the exact value is a hair above `m`. Round to nearest even is one add and one
/// shift: `(m + half - 1 + lsb) >> s`, with the sticky bit or-ed into bit 0 (it only matters when
/// the discarded part is exactly half, and bit 0 is below the half bit for `s >= 2`). No branch
/// depends on the operand data, which would mispredict about half the time.
#[inline(always)]
fn round_pack(neg: bool, e2: i32, m: u64, sticky: bool) -> u64 {
    let sign = u64::from(neg) << 39;
    let msb = 63 - m.leading_zeros() as i32;
    let shift = msb - 31;
    if shift < 2 {
        return round_pack_narrow(sign, e2 + msb, shift, m);
    }
    let s = shift as u32;
    let m2 = m | u64::from(sticky);
    let q = (m2 + ((1u64 << (s - 1)) - 1) + ((m2 >> s) & 1)) >> s;
    let carry = q >> 32; // 1 when rounding carried into a new bit
    let exp = e2 + msb + carry as i32;
    if (exp + 126) as u32 > 253 {
        return if exp > 127 { sign | (0xFF << 31) } else { sign };
    }
    sign | (((exp + 127) as u64) << 31) | ((q >> carry) & FRAC)
}

/// [`round_pack`] for the product of two 32 bit significands (`p` in `2^62 .. 2^64`, no sticky):
/// the top bit is bit 62 or 63, so the shift is 31 or 32 and no leading zero count is needed.
#[inline(always)]
fn round_pack_product(neg: bool, e2: i32, p: u64) -> u64 {
    let sign = u64::from(neg) << 39;
    let top = (p >> 63) as u32;
    let s = 31 + top;
    let q = (p + ((1u64 << (s - 1)) - 1) + ((p >> s) & 1)) >> s;
    let carry = q >> 32;
    let exp = e2 + 62 + top as i32 + carry as i32;
    if (exp + 126) as u32 > 253 {
        return if exp > 127 { sign | (0xFF << 31) } else { sign };
    }
    sign | (((exp + 127) as u64) << 31) | ((q >> carry) & FRAC)
}

/// [`round_pack`] when fewer than two bits are discarded (a deep cancellation: rare).
#[cold]
fn round_pack_narrow(sign: u64, exp: i32, shift: i32, m: u64) -> u64 {
    let mut exp = exp;
    let mut q = if shift > 0 {
        let s = shift as u32;
        let (rem, half) = (m & ((1u64 << s) - 1), 1u64 << (s - 1));
        let q = m >> s;
        q + u64::from(rem > half || (rem == half && q & 1 == 1))
    } else {
        m << (-shift) as u32
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
    sign | (((exp + 127) as u64) << 31) | (q & FRAC)
}

/// `Fx + Fy` (`sub` = `Fx - Fy`) for operands that are zero or normal, else `None`.
///
/// Zero operands (biased exponent 0, any fraction) are about 40 percent of the adds of the Bass
/// routines and most of the Farfisa weight sums; they take a branch that returns at once. The
/// zeros sit in fixed places (the held keys are the only non zero weights), so the branch predicts
/// well: measured on the oracle chords, branching beat a mask select by 35 percent (Farfisa 17.3
/// to 11.2 us per call) and 7 percent (Bass 29.0 to 26.9 us).
#[inline(always)]
pub(super) fn add(a: u64, b: u64, sub: bool) -> Option<u64> {
    if exp(a) == 255 || exp(b) == 255 {
        return None;
    }
    let b = (b ^ (u64::from(sub) << 39)) & ALL40; // `Fx - Fy` is `Fx + (-Fy)`
    if exp(a) == 0 || exp(b) == 0 {
        return Some(match (exp(a), exp(b)) {
            (0, 0) => (a & b) >> 39 << 39, // -0 only when both are negative
            (0, _) => b,
            _ => a & ALL40,
        });
    }
    let (neg_a, neg_b) = (a >> 39 == 1, b >> 39 == 1);
    // the larger magnitude first (the exponent and fraction fields order like the magnitude)
    let swap = a & MAG < b & MAG;
    let (hi, lo) = if swap { (b, a) } else { (a, b) };
    let (xn, yn) = if swap { (neg_b, neg_a) } else { (neg_a, neg_b) };
    let (xe, xm) = (exp(hi) as i32, (1 << 31) | (hi & FRAC));
    let (ye, ym) = (exp(lo) as i32, (1 << 31) | (lo & FRAC));
    // an alignment of 63 or more leaves only the sticky bit (`ym << 31` is below 2^63)
    let d = ((xe - ye) as u32).min(63);
    let (big, full) = (xm << 31, ym << 31);
    let small = full >> d;
    let sticky = full & ((1u64 << d) - 1) != 0;
    // `big + small` for equal signs, `big - small - sticky` otherwise, without a branch
    let diff = u64::from(xn != yn);
    let t = small + (u64::from(sticky) & diff);
    let m = big.wrapping_add((t ^ 0u64.wrapping_sub(diff)).wrapping_add(diff));
    if m == 0 && !sticky {
        return Some(0);
    }
    Some(round_pack(xn, xe - 127 - 62, m, sticky))
}

/// `float Rx by Ry` (`x` != 0): the integer `x` scaled by `2^scale`, rounded to 32 bits.
#[inline(always)]
pub(super) fn float_by(x: i32, scale: i32) -> u64 {
    round_pack(x < 0, scale, u64::from(x.unsigned_abs()), false)
}

/// `PASS Fx` (a denormal reads as zero, infinity and NaN give all ones).
#[inline(always)]
pub(super) fn pass(a: u64) -> u64 {
    match exp(a) {
        0 => a & (1 << 39),
        255 => ALL40,
        _ => a & ALL40,
    }
}

/// `Fx * Fy` for operands that are zero or normal, else `None` (zero: a signed zero).
#[inline(always)]
pub(super) fn mul(a: u64, b: u64) -> Option<u64> {
    if exp(a) == 255 || exp(b) == 255 {
        return None;
    }
    if exp(a) == 0 || exp(b) == 0 {
        return Some(((a ^ b) >> 39) << 39);
    }
    let (ma, mb) = ((1 << 31) | (a & FRAC), (1 << 31) | (b & FRAC));
    Some(round_pack_product(
        (a ^ b) >> 39 == 1,
        exp(a) as i32 + exp(b) as i32 - 254 - 62,
        ma * mb,
    ))
}

#[cfg(test)]
mod tests {
    use super::super::f40;

    /// xorshift64 (deterministic test data).
    fn next(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    fn operand(s: &mut u64, lo: u64, span: u64) -> u64 {
        let r = next(s);
        let e = lo + (r >> 40) % span;
        ((r & 1) << 39) | (e << 31) | ((r >> 8) & 0x7FFF_FFFF)
    }

    #[test]
    fn fast_paths_equal_the_general_routines() {
        let mut s = 0x1234_5678_9ABC_DEF1u64;
        for i in 0..400_000 {
            // exponents spread over the whole range, and close together (cancellation)
            let (lo, span) = if i % 2 == 0 { (1, 254) } else { (120, 12) };
            let (a, mut b) = (operand(&mut s, lo, span), operand(&mut s, lo, span));
            if i % 5 == 0 {
                b = (a ^ (1 << 39)) ^ (next(&mut s) & 0x7); // nearly equal magnitude, other sign
            }
            for sub in [false, true] {
                assert_eq!(
                    super::add(a, b, sub),
                    Some(f40::add_general(a, b, sub)),
                    "add {a:#x} {b:#x} {sub}"
                );
            }
            assert_eq!(
                super::mul(a, b),
                Some(f40::mul_general(a, b)),
                "mul {a:#x} {b:#x}"
            );
        }
    }

    #[test]
    fn zero_operands_equal_the_general_routines() {
        let mut s = 0x0F1E_2D3C_4B5A_6978u64;
        for i in 0..200_000 {
            let r = next(&mut s);
            // zero with any sign and fraction (a denormal reads as zero), or a normal value
            let zero = |r: u64| ((r & 1) << 39) | ((r >> 8) & 0x7FFF_FFFF);
            let a = if i % 3 == 0 {
                zero(r)
            } else {
                operand(&mut s, 1, 254)
            };
            let b = if i % 3 != 1 {
                zero(r >> 3)
            } else {
                operand(&mut s, 1, 254)
            };
            for sub in [false, true] {
                assert_eq!(
                    super::add(a, b, sub),
                    Some(f40::add_general(a, b, sub)),
                    "add {a:#x} {b:#x} {sub}"
                );
            }
            assert_eq!(
                super::mul(a, b),
                Some(f40::mul_general(a, b)),
                "mul {a:#x} {b:#x}"
            );
        }
        // infinity and NaN with a zero stay on the general path
        assert_eq!(super::add(0, 0xFF << 31, false), None);
        assert_eq!(super::mul(0, 0xFF << 31), None);
    }

    #[test]
    fn float_by_and_pass_equal_the_general_routines() {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..200_000 {
            let r = next(&mut s);
            let x = (r >> 20) as i32 >> ((r & 31) as u32); // magnitudes of every width
            let scale = ((r >> 8) & 0xFF) as i32 - 200;
            if x != 0 {
                assert_eq!(super::float_by(x, scale), f40::float_by_general(x, scale));
            }
            let v = (r & 1) << 39 | (r >> 11 & 0xFF) << 31 | (next(&mut s) & 0x7FFF_FFFF);
            assert_eq!(super::pass(v), f40::pass_general(v), "pass {v:#x}");
        }
    }

    #[test]
    fn special_classes_fall_back() {
        let one = 0x7Fu64 << 31;
        assert_eq!(super::add(0, one, false), Some(one));
        assert_eq!(super::mul(one, 0xFF << 31), None);
        assert_eq!(super::add(one, one, true), Some(0));
    }
}
