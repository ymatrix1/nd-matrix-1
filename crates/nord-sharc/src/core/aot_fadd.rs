//! `Fx + Fy` for two normal 40-bit floats, bit-exact with `float::add` but mostly on the host's
//! double-precision unit.
//!
//! A register float is `s | e8 | f31` (32-bit significand). Shifting the 39 magnitude bits left by
//! 21 puts `e8` in the low 8 bits of the double's exponent field and `f31` in the top of its
//! fraction, so the double is the register value times `2^-896` (exact, normal for `e8 >= 1`);
//! both operands carry the same scale, so a double add of the two is the register add up to the
//! one rounding. Its 53-bit result `s` is then rounded to the register's 32 (or, with RND32, 24)
//! significant bits by integer rounding on the double's bits; a carry out of the fraction
//! increments the exponent field by itself.
//!
//! Why that equals one correct rounding of the exact sum `v`: `s` is the double nearest to `v`
//! and every rounding boundary of the register format is a double, so `s` and `v` round alike
//! unless `s` sits exactly on a boundary. Round-to-nearest boundaries are midpoints (`low ==
//! half`), truncation boundaries are representable values (`low == 0`). Those cases are decided
//! by the exact integer kernel unless the sum is known to be exact in a double: operands with
//! 32-bit significands and exponents at most 20 apart sum in at most 53 bits.
//!
//! Provenance: ASSUMED correct by the argument above (needs IEEE-754 round-to-nearest double
//! addition, i.e. SSE2 or AArch64, not x87); VERIFIED bit-identical to `float::add` by the sweeps
//! in `aot_fast.rs` and `aot_fast_sweep.rs` (flags and value, every TRUNC x RND32 mode).
use super::aot_fast::significand;
use super::aot_round::{exp_field, pack, round_norm, unpack, Packed};
use super::float::{Mode, Out};

/// Largest exponent gap for which the sum of two 32-bit significands fits 53 bits (the check
/// `gap + EXACT_GAP > 2 * EXACT_GAP` in wrapping `u64` arithmetic is `|gap| > EXACT_GAP`).
const EXACT_GAP: u64 = 20;
const MAG39: u64 = (1 << 39) - 1;

/// The register float `a` (normal) as a double, scaled by `2^-896`.
#[inline(always)]
fn scaled(a: u64) -> f64 {
    f64::from_bits(((a << 21) & ((1 << 60) - 1)) | ((a << 24) & (1 << 63)))
}

/// `a + b` for normal `a` and `b` (the sign of `b` already includes a subtraction).
#[inline(always)]
pub(super) fn add_normal(a: u64, b: u64, mode: Mode) -> Out {
    // (discarded-bit mask, half, lowest kept bit, field mask) of the double's 52-bit fraction
    let (low_mask, half, odd_bit, fmask) = if mode.r32 {
        ((1 << 29) - 1, 1 << 28, 1 << 29, MAG39 & !0xFF)
    } else {
        ((1 << 21) - 1, 1 << 20, 1 << 21, MAG39)
    };
    let r = (scaled(a) + scaled(b)).to_bits();
    let low = r & low_mask;
    // all ones for round-to-nearest, zero for truncation
    let nearest = u64::from(!mode.trunc).wrapping_neg();
    // the rounding boundary an inexact double sum must not land on: the midpoint when rounding to
    // nearest, the representable value itself when truncating
    let boundary = half & nearest;
    let gap = exp_field(a)
        .wrapping_sub(exp_field(b))
        .wrapping_add(EXACT_GAP);
    let undecided = (gap > 2 * EXACT_GAP) & (low == boundary);
    let up = (half - 1 + u64::from(r & odd_bit != 0)) & nearest;
    let r = r + up;
    // biased exponent of the result; 0 (zero or flush) and 255.. (overflow) are not normal
    let field = (r >> 52) & 0x7FF;
    if undecided | (field.wrapping_sub(1) >= 254) {
        return unpack(add_exact(a, b, mode));
    }
    Out {
        v: ((r >> 21) & fmask) | ((r >> 24) & (1 << 39)),
        an: r >> 63 == 1,
        ..Out::default()
    }
}

/// The exact integer kernel for the cases `add_normal` does not decide: significands aligned and
/// added in 64 bits, normalized with a leading-zero count and rounded once.
#[inline(never)]
fn add_exact(a: u64, b: u64, mode: Mode) -> Packed {
    pack(add_exact_out(a, b, mode))
}

fn add_exact_out(a: u64, b: u64, mode: Mode) -> Out {
    let swap = (b & MAG39) > (a & MAG39);
    let (hi, lo) = if swap { (b, a) } else { (a, b) };
    let (n_hi, n_lo) = (hi >> 39, lo >> 39);
    let d = ((exp_field(hi) - exp_field(lo)) as u32).min(62);
    let big = significand(hi) << 30;
    let full = significand(lo) << 30;
    let small = full >> d;
    // bits shifted out: `full` is non-zero, so some are lost exactly when it has fewer than `d`
    // trailing zeros (for d = 62 that is always, as `full` < 2^62)
    let sticky = full.trailing_zeros() < d;
    // same signs add `small`; opposite signs subtract it, and one more when bits were lost
    let m = if n_hi == n_lo {
        big + small
    } else {
        big - small - u64::from(sticky)
    };
    if m == 0 {
        // exact cancellation; `sticky` implies a gap over 30 and so a non-zero `m`
        return Out::value(0, false, true);
    }
    let lz = m.leading_zeros();
    // bit 63 of `m << lz` has the weight 2^(e_hi - 127 - 61 + 63 - lz)
    let exp = exp_field(hi) as i64 - 125 - i64::from(lz);
    round_norm(n_hi == 1, exp, m << lz, sticky, mode, mode.r32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::float;

    fn f40(neg: bool, exp: u64, frac: u64) -> u64 {
        u64::from(neg) << 39 | exp << 31 | frac & 0x7FFF_FFFF
    }

    #[test]
    fn scaled_doubles_keep_the_register_value_times_a_power_of_two() {
        let one = scaled(f40(false, 127, 0));
        let one_and_half = scaled(f40(false, 127, 1 << 30));
        assert_eq!(one_and_half / one, 1.5);
        assert_eq!(scaled(f40(true, 127, 0)), -one);
        assert_eq!(scaled(f40(false, 128, 0)) / one, 2.0);
    }

    #[test]
    fn double_path_and_exact_kernel_agree_with_the_reference_on_boundary_sums() {
        let ops = [
            f40(false, 127, 0),
            f40(false, 127, 0x7FFF_FFFF),
            f40(true, 127, 0x7FFF_FFFF),
            f40(false, 100, 1),
            f40(false, 147, 1),
            f40(false, 148, 0x4000_0000),
            f40(true, 148, 0x4000_0001),
            f40(false, 1, 0),
            f40(false, 254, 0x7FFF_FFFF),
        ];
        let modes =
            [(false, false), (true, false), (false, true), (true, true)].map(|(trunc, r32)| Mode {
                trunc,
                r32,
                sat: false,
            });
        for mode in modes {
            for &a in &ops {
                for &b in &ops {
                    let keep = if mode.r32 { !0xFF } else { !0 };
                    let (a, b) = (a & keep, b & keep);
                    let want = float::add(a, b, false, false, mode);
                    // the exact kernel is a valid answer for any normal pair
                    assert_eq!(
                        add_exact_out(a, b, mode),
                        want,
                        "exact {a:#x} {b:#x} {mode:?}"
                    );
                    assert_eq!(add_normal(a, b, mode), want, "dbl {a:#x} {b:#x} {mode:?}");
                }
            }
        }
    }
}
