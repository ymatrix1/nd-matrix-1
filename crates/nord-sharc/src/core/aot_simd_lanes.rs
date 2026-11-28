//! The SSE2 lane code of the paired float kernels (`aot_simd.rs`): the normal-range paths of
//! `aot_fadd.rs` / `aot_fast.rs` written for two 64-bit lanes (`addpd` for the double sum,
//! `pmuludq` for the 32 x 32 significand product, 64-bit shifts, masks and adds for the rounding),
//! plus the zero-operand forms of `aot_round.rs` (`add_with_zero`, `mul_with_zero`): a lane whose
//! operand is zero or denormal never needs the double path, and real passes are mostly zeros.
//!
//! [`add`] and [`mul`] return the result value of every lane, the lanes they could not decide
//! (`bad`: infinity or NaN operand, RND32 sum with a zero operand, overflow or flush, a double sum
//! on a rounding boundary that is not known exact) and the lanes of the zero form (`flag`: both
//! operands zero for a sum, any zero operand for a product). SSE2 is the x86-64 baseline, so the
//! intrinsics need no feature detection; other targets reject every lane (`bad = 3`) and the
//! caller uses the scalar kernel.
//!
//! Provenance: VERIFIED against the scalar kernels and the reference by `aot_simd_sweep.rs`; the
//! rounding constants are those of `aot_fadd.rs` and `aot_round.rs`.

/// Lane results: value per lane, rejected lanes (bit 0 PEx, bit 1 PEy) and zero-form lanes.
pub(super) struct Lanes {
    pub(super) v: [u64; 2],
    pub(super) bad: u32,
    pub(super) flag: u32,
}

#[cfg(target_arch = "x86_64")]
pub(super) use x86::{add, mul};

#[cfg(not(target_arch = "x86_64"))]
pub(super) use other::{add, mul};

#[cfg(target_arch = "x86_64")]
mod x86 {
    use super::Lanes;
    use crate::core::float::Mode;
    use core::arch::x86_64::*;

    const MAG39: u64 = (1 << 39) - 1;

    /// Wraps the intrinsics of one helper: SSE2 is part of the x86-64 baseline, the only feature
    /// they need.
    macro_rules! sse2 {
        ($($body:tt)*) => {
            // SAFETY: every x86-64 target has SSE2.
            unsafe { $($body)* }
        };
    }

    #[inline(always)]
    fn set(x: u64) -> __m128i {
        sse2! { _mm_set1_epi64x(x as i64) }
    }

    #[inline(always)]
    fn load(v: [u64; 2]) -> __m128i {
        sse2! { _mm_set_epi64x(v[1] as i64, v[0] as i64) }
    }

    #[inline(always)]
    fn store(v: __m128i) -> [u64; 2] {
        sse2! { [_mm_cvtsi128_si64(v) as u64, _mm_cvtsi128_si64(_mm_unpackhi_epi64(v, v)) as u64] }
    }

    /// Replicates the low dword of each lane into the whole lane (a 32-bit compare result becomes
    /// a lane mask).
    #[inline(always)]
    fn lane(c: __m128i) -> __m128i {
        sse2! { _mm_shuffle_epi32::<0xA0>(c) }
    }

    /// Bit mask (bit 0: lane 0) of a lane mask.
    #[inline(always)]
    fn bits(c: __m128i) -> u32 {
        sse2! { _mm_movemask_pd(_mm_castsi128_pd(c)) as u32 }
    }

    /// Exponent field (bits 38..31) of each lane.
    #[inline(always)]
    fn exp(x: __m128i) -> __m128i {
        sse2! { _mm_and_si128(_mm_srli_epi64::<31>(x), set(0xFF)) }
    }

    /// Lane mask: low dword equal to `k`.
    #[inline(always)]
    fn is(e: __m128i, k: i32) -> __m128i {
        sse2! { lane(_mm_cmpeq_epi32(e, _mm_set1_epi32(k))) }
    }

    /// Lane mask: signed compare `x > k` on the low dword of each lane.
    #[inline(always)]
    fn gt(x: __m128i, k: i32) -> __m128i {
        sse2! { lane(_mm_cmpgt_epi32(x, _mm_set1_epi32(k))) }
    }

    /// Lane mask: the low dword of `x` is negative.
    #[inline(always)]
    fn lt0(x: __m128i) -> __m128i {
        sse2! { lane(_mm_cmpgt_epi32(_mm_setzero_si128(), x)) }
    }

    #[inline(always)]
    fn or(a: __m128i, b: __m128i) -> __m128i {
        sse2! { _mm_or_si128(a, b) }
    }

    #[inline(always)]
    fn and(a: __m128i, b: __m128i) -> __m128i {
        sse2! { _mm_and_si128(a, b) }
    }

    /// `!a & b`.
    #[inline(always)]
    fn andn(a: __m128i, b: __m128i) -> __m128i {
        sse2! { _mm_andnot_si128(a, b) }
    }

    #[inline(always)]
    fn add64(a: __m128i, b: __m128i) -> __m128i {
        sse2! { _mm_add_epi64(a, b) }
    }

    #[inline(always)]
    fn sub64(a: __m128i, b: __m128i) -> __m128i {
        sse2! { _mm_sub_epi64(a, b) }
    }

    #[inline(always)]
    fn xor(a: __m128i, b: __m128i) -> __m128i {
        sse2! { _mm_xor_si128(a, b) }
    }

    #[inline(always)]
    fn shr<const N: i32>(a: __m128i) -> __m128i {
        sse2! { _mm_srli_epi64::<N>(a) }
    }

    #[inline(always)]
    fn shl<const N: i32>(a: __m128i) -> __m128i {
        sse2! { _mm_slli_epi64::<N>(a) }
    }

    /// `x` with the 8 extension bits cleared when RND32 is set.
    #[inline(always)]
    fn keep(x: [u64; 2], mode: Mode) -> __m128i {
        and(load(x), set(!(0xFFu64 * u64::from(mode.r32))))
    }

    /// The register float as a double scaled by `2^-896` (see `aot_fadd.rs`).
    #[inline(always)]
    fn scaled(x: __m128i) -> __m128d {
        let mag = and(shl::<21>(x), set((1 << 60) - 1));
        let sign = and(shl::<24>(x), set(1 << 63));
        sse2! { _mm_castsi128_pd(or(mag, sign)) }
    }

    /// `a + b` (`b`'s sign includes a subtraction).
    #[inline(always)]
    pub(in super::super) fn add(a: [u64; 2], b: [u64; 2], sub: bool, mode: Mode) -> Lanes {
        // (discarded-bit mask, half, odd-bit position, field mask), as `add_normal`
        let (low_mask, half, odd_pos, fmask) = if mode.r32 {
            ((1u64 << 29) - 1, 1u64 << 28, 29, MAG39 & !0xFF)
        } else {
            ((1u64 << 21) - 1, 1u64 << 20, 21, MAG39)
        };
        let nearest = set(u64::from(!mode.trunc).wrapping_neg());
        let (a, b) = (keep(a, mode), keep(b, mode));
        let b = xor(b, set(u64::from(sub) << 39));
        let (ea, eb) = (exp(a), exp(b));
        let r = sse2! { _mm_castpd_si128(_mm_add_pd(scaled(a), scaled(b))) };
        let low = and(r, set(low_mask));
        let boundary = and(set(half), nearest);
        // `gap = ea - eb + 20` is outside 0..=40: a double sum on a boundary may be inexact
        let gap = add64(sub64(ea, eb), set(20));
        let wide = or(gt(gap, 40), lt0(gap));
        let undecided = and(wide, sse2! { lane(_mm_cmpeq_epi32(low, boundary)) });
        let odd = sse2! { _mm_srl_epi64(and(r, set(1 << odd_pos)), _mm_cvtsi32_si128(odd_pos)) };
        let r = add64(r, and(add64(set(half - 1), odd), nearest));
        let field = and(shr::<52>(r), set(0x7FF));
        let bad_field = or(is(field, 0), gt(field, 254));
        let v = or(
            and(shr::<21>(r), set(fmask)),
            and(shr::<24>(r), set(1 << 39)),
        );
        // zero or denormal operands (flushed): the other operand passes through, two zeros give
        // a zero whose sign is negative only when both are
        let (za, zb) = (is(ea, 0), is(eb, 0));
        let (both, only_a, only_b) = (and(za, zb), andn(zb, za), andn(za, zb));
        let neg0 = shl::<39>(shr::<39>(and(a, b)));
        let v = or(
            or(and(both, neg0), and(only_a, b)),
            or(and(only_b, a), andn(or(za, zb), v)),
        );
        let inf = or(is(ea, 255), is(eb, 255));
        // RND32 sums with a zero operand go to the scalar kernel (the reference rounds them)
        let zero_bad = if mode.r32 { or(za, zb) } else { set(0) };
        let bad = or(
            or(inf, zero_bad),
            andn(or(za, zb), or(bad_field, undecided)),
        );
        Lanes {
            v: store(v),
            bad: bits(bad),
            flag: bits(both),
        }
    }

    /// `a * b`.
    #[inline(always)]
    pub(in super::super) fn mul(a: [u64; 2], b: [u64; 2], mode: Mode) -> Lanes {
        let (sh, half_m1, fmask) = if mode.r32 {
            (40, (1u64 << 39) - 1, 0x7FFF_FF00)
        } else {
            (32, (1u64 << 31) - 1, 0x7FFF_FFFF)
        };
        let nearest = set(u64::from(!mode.trunc).wrapping_neg());
        let (a, b) = (keep(a, mode), keep(b, mode));
        let (ea, eb) = (exp(a), exp(b));
        let neg = shr::<39>(xor(a, b));
        let sig = |x| or(set(1 << 31), and(x, set(0x7FFF_FFFF)));
        let p = sse2! { _mm_mul_epu32(sig(a), sig(b)) };
        // top bit at 63 or 62: `p + p` normalizes the second case
        let low = xor(shr::<63>(p), set(1));
        let p = add64(p, and(p, sub64(set(0), low)));
        let e = sub64(add64(add64(ea, eb), set(-253i64 as u64)), low);
        let t = and(sse2! { _mm_srl_epi64(p, _mm_cvtsi32_si128(sh)) }, set(1));
        let sum = add64(p, and(add64(set(half_m1), t), nearest));
        // `p` has bit 63 set, so the sum wrapped (a carry) exactly when its bit 63 is clear
        let e = add64(e, xor(shr::<63>(sum), set(1)));
        let x = add64(e, set(126));
        let bad_exp = or(gt(x, 253), lt0(x));
        let v = or(
            or(shl::<39>(neg), shl::<31>(add64(e, set(127)))),
            and(shr::<32>(sum), set(fmask)),
        );
        // a zero or denormal operand gives a signed zero
        let zero = or(is(ea, 0), is(eb, 0));
        let v = or(and(zero, shl::<39>(neg)), andn(zero, v));
        let bad = or(or(is(ea, 255), is(eb, 255)), andn(zero, bad_exp));
        Lanes {
            v: store(v),
            bad: bits(bad),
            flag: bits(zero),
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
mod other {
    use super::Lanes;
    use crate::core::float::Mode;

    /// No vector path on this target: every lane is left to the scalar kernel.
    pub(in super::super) fn add(_: [u64; 2], _: [u64; 2], _: bool, _: Mode) -> Lanes {
        Lanes {
            v: [0; 2],
            bad: 3,
            flag: 0,
        }
    }

    pub(in super::super) fn mul(_: [u64; 2], _: [u64; 2], _: Mode) -> Lanes {
        Lanes {
            v: [0; 2],
            bad: 3,
            flag: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::float::Mode;

    const ONE: u64 = 127 << 31;

    #[test]
    fn lane_masks_name_the_rejected_and_the_zero_form_lanes() {
        let m = Mode::default();
        // lane 0 ordinary, lane 1 infinity: only lane 1 is rejected
        let l = add([ONE, ONE], [ONE, 255 << 31], false, m);
        assert_eq!((l.bad, l.flag), (2, 0));
        // zero + zero is the zero form, zero + normal is decided without the flag
        let l = add([0, 0], [0, ONE], false, m);
        assert_eq!((l.bad, l.flag), (0, 1));
        // a product with a zero operand is the zero form in that lane only
        let l = mul([0, ONE], [ONE, ONE], m);
        assert_eq!((l.bad, l.flag), (0, 1));
        // RND32 sums with a zero operand, and overflow, are left to the scalar kernel
        let r32 = Mode { r32: true, ..m };
        assert_eq!(add([0, ONE], [ONE, ONE], false, r32).bad, 1);
        assert_eq!(mul([254 << 31, ONE], [254 << 31, ONE], m).bad, 1);
    }
}
