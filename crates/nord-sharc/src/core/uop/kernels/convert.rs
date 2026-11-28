//! Conversions: `FLOAT`, `FIX`/`TRUNC`, `RND` and `SCALB`.
//!
//! OWNER: W1-KERN. Integer results come back as a 32-bit value in the low bits of the [`F40`].
//! The reference (`float_conv.rs`) rounds in `u128`; the bodies here are
//!
//! | Kernel | Body |
//! |---|---|
//! | `ffloat` | `aot_fast::fast_from_int` (leading-zero count, `round_norm`) |
//! | `ffix` | `u64` shift and round on the 32-bit significand; a shift above 33 changes nothing (the significand is below 2^32), so it is clamped |
//! | `frnd` | `round_norm` with the 24-bit boundary on the significand left-aligned in a `u64` |
//! | `fscalb` | one exponent add and a range check, no rounding |
//!
//! Semantics kept from the reference (all part of the grid): a NaN gives all ones with AI only,
//! infinity and an out-of-range `FIX` overflow (saturated with ALUSAT, all ones with AV and AI
//! without), `FIX` of a magnitude below one latches AU, a denormal is a zero, RND32 clears the
//! extension first.
//!
//! Provenance: VERIFIED bit-identical to `float_conv::{from_int, to_int, rnd, scalb}` (value and
//! every flag) by `convert_equals_reference_on_the_class_grid_and_seeded_streams`.
use super::operand::{opnd, INF_MAG};
use super::{finish, FlagsOut, Mode, F40};
use crate::core::aot_fast::fast_from_int;
use crate::core::aot_round::round_norm;
use crate::core::float::{overflow_value, ALL_ONES};

/// Mantissa field of a register float, bits 30..0 (the hidden bit is not stored).
/// VERIFIED: `float.rs` layout, checked by the convert tests.
const FRAC_MASK: u64 = 0x7FFF_FFFF;
/// The implicit leading bit of the 32-bit significand.
/// VERIFIED: `float::unpack` (`1 << 31 | frac`), checked by the convert tests.
const HIDDEN: u64 = 1 << 31;
/// Bias of the exponent field.
/// VERIFIED: `float::unpack` (`exp = field - 127`), checked by the convert tests.
const BIAS: i64 = 127;
/// Shifts of a 32-bit significand at or above this give the same quotient (0), remainder and
/// halfway point comparison.
/// VERIFIED: equality with the `u128` reference for every scale from -400 to 400 on the grid.
const SHIFT_CAP: i64 = 33;

/// The NaN result of `FIX`: all ones, AI only.
/// VERIFIED: `float_conv::to_int` (simulator: AI only, no AN), checked by the convert tests.
fn nan_int(st: &mut FlagsOut) -> F40 {
    *st = FlagsOut {
        ai: true,
        ..FlagsOut::default()
    };
    0xFFFF_FFFF
}

/// The NaN result of `RND` and `SCALB`: the 40-bit all-ones value with AI only.
fn nan_float(st: &mut FlagsOut) -> F40 {
    *st = FlagsOut {
        ai: true,
        ..FlagsOut::default()
    };
    ALL_ONES
}

/// A zero (az) or infinity of sign `neg` that passes through unchanged.
fn pass_special(neg: bool, inf: bool, st: &mut FlagsOut) -> F40 {
    *st = FlagsOut {
        az: !inf,
        an: neg,
        ..FlagsOut::default()
    };
    u64::from(neg) << 39 | if inf { INF_MAG } else { 0 }
}

/// `Fn = FLOAT x BY scale`.
#[inline(always)]
pub fn ffloat(x: i32, scale: i32, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(fast_from_int(x, scale, mode), st)
}

/// `Rn = FIX a BY scale`, or `TRUNC` with `toward_zero`.
#[inline(always)]
pub fn ffix(a: F40, scale: i32, toward_zero: bool, mode: Mode, st: &mut FlagsOut) -> F40 {
    let x = opnd(a, mode.r32);
    // out of range: saturated with ALUSAT, otherwise all ones with AV and AI
    let overflow = |neg: bool, st: &mut FlagsOut| {
        let v: u64 = match (mode.sat, neg) {
            (false, _) => 0xFFFF_FFFF,
            (true, true) => 0x8000_0000,
            (true, false) => 0x7FFF_FFFF,
        };
        *st = FlagsOut {
            an: mode.sat && v >> 31 == 1,
            av: true,
            ai: !mode.sat,
            ..FlagsOut::default()
        };
        v
    };
    if x.nan() {
        return nan_int(st);
    }
    if x.mag == INF_MAG {
        return overflow(x.neg, st);
    }
    if x.mag == 0 {
        *st = FlagsOut {
            az: true,
            ..FlagsOut::default()
        };
        return 0;
    }
    let e = (x.mag >> 31) as i64 - BIAS + i64::from(scale);
    let man = HIDDEN | x.mag & FRAC_MASK;
    // 2^31 is only reachable as -2^31: exponent 31 with an empty fraction and a minus sign
    if e >= 31 && !(e == 31 && x.neg && x.mag & FRAC_MASK == 0) {
        return overflow(x.neg, st);
    }
    let shift = (31 - e).min(SHIFT_CAP) as u32;
    let (mut q, rem, half) = (
        man >> shift,
        man & ((1u64 << shift) - 1),
        (1u64 << shift) >> 1,
    );
    let up = if toward_zero {
        false
    } else if mode.trunc {
        x.neg && rem != 0
    } else {
        rem > half || (rem == half && q & 1 == 1)
    };
    q += u64::from(up);
    if q > if x.neg { 0x8000_0000 } else { 0x7FFF_FFFF } {
        return overflow(x.neg, st);
    }
    let v = if x.neg {
        (q as u32).wrapping_neg()
    } else {
        q as u32
    };
    *st = FlagsOut {
        az: v == 0,
        an: v >> 31 == 1,
        // a non-zero input whose scaled magnitude is below one latches AUS (VERIFIED, 2156 rows
        // of `float_measured.rs`, as in the reference)
        au: e < 0,
        ..FlagsOut::default()
    };
    u64::from(v)
}

/// `RND a`: round to a 32-bit boundary.
#[inline(always)]
pub fn frnd(a: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    let x = opnd(a, mode.r32);
    if x.nan() {
        return nan_float(st);
    }
    if x.mag == 0 || x.mag == INF_MAG {
        return pass_special(x.neg, x.mag != 0, st);
    }
    let exp = (x.mag >> 31) as i64 - BIAS;
    // the significand left-aligned: bit 63 has the weight 2^exp
    finish(
        round_norm(
            x.neg,
            exp,
            1 << 63 | (x.mag & FRAC_MASK) << 32,
            false,
            mode,
            true,
        ),
        st,
    )
}

/// `SCALB a BY n`.
#[inline(always)]
pub fn fscalb(a: F40, n: i32, mode: Mode, st: &mut FlagsOut) -> F40 {
    let x = opnd(a, mode.r32);
    if x.nan() {
        return nan_float(st);
    }
    if x.mag == 0 || x.mag == INF_MAG {
        return pass_special(x.neg, x.mag != 0, st);
    }
    let e = (x.mag >> 31) as i64 - BIAS + i64::from(n);
    let sign = u64::from(x.neg) << 39;
    if e > 127 {
        *st = FlagsOut {
            an: x.neg,
            av: true,
            ..FlagsOut::default()
        };
        overflow_value(x.neg, mode, mode.r32)
    } else if e < -126 {
        *st = FlagsOut {
            az: true,
            an: x.neg,
            au: true,
            ..FlagsOut::default()
        };
        sign
    } else {
        *st = FlagsOut {
            an: x.neg,
            ..FlagsOut::default()
        };
        sign | ((e + BIAS) as u64) << 31 | x.mag & FRAC_MASK
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::float_conv;
    use crate::core::uop::kernels::f40;
    use crate::core::uop::kernels::grid::{self, Seen, AI, AN, AU, AV, AZ};
    use crate::core::uop::verify::gen::Rng;

    #[test]
    fn conversions_round_trip_small_integers_and_scale_by_powers_of_two() {
        let (mut st, m) = (FlagsOut::default(), Mode::default());
        assert_eq!(ffloat(5, 0, m, &mut st), f40(5.0));
        assert_eq!(ffix(f40(5.0), 0, true, m, &mut st), 5);
        assert_eq!(ffix(f40(-5.5), 0, true, m, &mut st) as u32, (-5i32) as u32);
        assert_eq!(fscalb(f40(3.0), 2, m, &mut st), f40(12.0));
        assert_eq!(frnd(f40(1.25), m, &mut st), f40(1.25));
    }

    /// Scales that put the binary point at, inside and far outside the 32-bit integer range.
    fn scales(r: &mut Rng) -> [i32; 12] {
        [
            0,
            1,
            -1,
            31,
            -31,
            32,
            -32,
            127,
            -127,
            (r.next_u64() % 120) as i32 - 60,
            (r.next_u64() % 800) as i32 - 400,
            [i32::MIN, i32::MAX, 0x7FFF_FFF0][(r.next_u64() % 3) as usize],
        ]
    }

    fn check_fix(a: F40, scale: i32, tz: bool, mode: Mode, seen: &mut Seen) {
        let mut st = grid::dirty();
        let v = ffix(a, scale, tz, mode, &mut st);
        let want = float_conv::to_int(a, scale, tz, mode);
        assert_eq!(
            st.out(v),
            want,
            "fix a={a:#x} scale={scale} tz={tz} {mode:?}"
        );
        seen.result(&st);
        seen.operand(a);
    }

    fn check_rnd_scalb(a: F40, n: i32, mode: Mode, seen: &mut Seen) {
        let mut st = grid::dirty();
        let v = frnd(a, mode, &mut st);
        assert_eq!(st.out(v), float_conv::rnd(a, mode), "rnd a={a:#x} {mode:?}");
        seen.result(&st);
        let v = fscalb(a, n, mode, &mut st);
        assert_eq!(
            st.out(v),
            float_conv::scalb(a, n, mode),
            "scalb a={a:#x} n={n} {mode:?}"
        );
        seen.result(&st);
        seen.operand(a);
    }

    #[test]
    fn convert_equals_reference_on_the_class_grid_and_seeded_streams() {
        let mut seen = Seen::default();
        let mut r = Rng::new(0xC0DE_0001);
        let g = grid::grid();
        for mode in grid::modes() {
            for &a in &g {
                for scale in scales(&mut r) {
                    for tz in [false, true] {
                        check_fix(a, scale, tz, mode, &mut seen);
                    }
                    check_rnd_scalb(a, scale, mode, &mut seen);
                }
            }
        }
        for mode in grid::modes() {
            for _ in 0..30_000 {
                let a = grid::operand(&mut r);
                let scale = (r.next_u64() % 90) as i32 - 50;
                check_fix(a, scale, r.next_u64() & 1 == 1, mode, &mut seen);
                check_rnd_scalb(a, scale, mode, &mut seen);
                // FIX of a value exactly half way between two integers, just above and just below
                let k = r.next_u64() % 31;
                let sign = (r.next_u64() & 1) << 39;
                let hi = (r.next_u64() & ((1 << k) - 1)) << (31 - k);
                let tie = sign | (127 + k) << 31 | hi | 1 << (30 - k);
                for v in [tie, tie | 1, tie - 1, tie ^ 1 << (30 - k), tie | 0x80] {
                    check_fix(v, 0, false, mode, &mut seen);
                    check_fix(v, 0, true, mode, &mut seen);
                }
            }
        }
        seen.assert_reached(400_000, &[AZ, AN, AV, AI, AU]);
    }

    #[test]
    fn float_equals_reference_for_integers_and_scales() {
        let mut seen = Seen::default();
        let mut r = Rng::new(0xC0DE_0002);
        let mut ints = vec![
            0,
            1,
            -1,
            2,
            3,
            i32::MAX,
            i32::MIN,
            i32::MIN + 1,
            0x7FFF_FF80,
            0x0100_0001,
        ];
        for _ in 0..8_000 {
            ints.push(match r.next_u64() % 3 {
                0 => r.next_u32() as i32,
                1 => (r.next_u32() as i32) >> (r.next_u64() % 32),
                _ => (r.next_u64() % 9) as i32 - 4,
            });
        }
        for mode in grid::modes() {
            for &x in &ints {
                for scale in scales(&mut r) {
                    let mut st = grid::dirty();
                    let v = ffloat(x, scale, mode, &mut st);
                    assert_eq!(
                        st.out(v),
                        float_conv::from_int(x, scale, mode),
                        "float x={x} scale={scale} {mode:?}"
                    );
                    seen.result(&st);
                }
            }
        }
        // zero (AZ), a negative value (AN), overflow of the exponent (AV) and flush (AU)
        assert!(seen.cases > 100_000 && seen.flags[AZ] > 0 && seen.flags[AN] > 0);
        assert!(seen.flags[AV] > 0 && seen.flags[AU] > 0, "{seen:?}");
    }

    #[test]
    fn fix_edges_saturate_round_and_latch_underflow_like_the_reference() {
        let sat = Mode {
            sat: true,
            ..Mode::default()
        };
        let (mut st, m) = (grid::dirty(), Mode::default());
        // -2^31 is the one exact value at exponent 31; +2^31 overflows
        assert_eq!(
            ffix(1 << 39 | (127 + 31) << 31, 0, false, m, &mut st),
            0x8000_0000
        );
        assert!(!st.av && st.an);
        assert_eq!(ffix((127 + 31) << 31, 0, false, sat, &mut st), 0x7FFF_FFFF);
        assert!(st.av && !st.ai);
        assert_eq!(ffix((127 + 31) << 31, 0, false, m, &mut st), 0xFFFF_FFFF);
        assert!(st.av && st.ai && !st.an);
        // 0.5 rounds to even (0) and latches AU; 1.5 rounds to 2
        assert_eq!(ffix(f40(0.5), 0, false, m, &mut st), 0);
        assert!(st.au && st.az);
        assert_eq!(ffix(f40(1.5), 0, false, m, &mut st), 2);
        assert_eq!(ffix(f40(2.5), 0, false, m, &mut st), 2);
        assert!(!st.au);
        // TRUNC mode rounds FIX toward minus infinity
        let t = Mode { trunc: true, ..m };
        assert_eq!(ffix(f40(-0.25), 0, false, t, &mut st) as u32, u32::MAX);
        assert_eq!(ffix(f40(0.25), 0, false, t, &mut st), 0);
    }
}
