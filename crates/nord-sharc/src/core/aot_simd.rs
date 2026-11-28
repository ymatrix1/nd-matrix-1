//! Paired float kernels for the SIMD compute forms: `Fx + Fy`, `Fx - Fy` and `Fx * Fy` on the PEx
//! and PEy operands together ([`fast_add2`], [`fast_sub2`], [`fast_mul2`]). Both elements of a
//! SIMD-mode instruction run the same op with the same MODE1, so the pair is one 2 x 64-bit vector.
//!
//! The vector code is the normal-range path of `aot_fadd.rs` / `aot_fast.rs` written lane for
//! lane with SSE2 (the x86-64 baseline: `addpd` for the double sum, `pmuludq` for the 32 x 32
//! significand product, 64-bit shifts, masks and adds for the rounding), so it needs no runtime
//! feature detection and no allocation. Every lane the vector path cannot decide (an operand that
//! is zero, denormal, infinity or NaN, a result that overflows or flushes, an add whose double sum
//! sits on a rounding boundary) is flagged in a lane mask and recomputed by the exact scalar
//! kernel of `aot_fast.rs` on the original operands, so the other lane is unaffected. Targets
//! other than x86-64 use the scalar kernel for both lanes (`*_scalar`); there is no NEON path
//! (it could not be built or tested on the development host).
//!
//! Provenance: VERIFIED bit-identical (every `Out` field) to `float::add` / `float::mul` and to
//! the scalar kernels by `aot_simd_sweep.rs` (random operand classes, every exponent gap, tie and
//! overflow/flush edges, lane isolation, all four TRUNC x RND32 modes); the rounding constants are
//! those of `aot_fadd.rs` and `aot_round.rs`, which carry their own provenance.
use super::aot_fast::{fast_add, fast_mul};
use super::aot_simd_lanes as lanes;
use super::float::{Mode, Out};

/// `Fx + Fy` for both elements, equal to `[fast_add(a[0], b[0], ..), fast_add(a[1], b[1], ..)]`.
#[inline(always)]
pub(super) fn fast_add2(a: [u64; 2], b: [u64; 2], sub: bool, mode: Mode) -> [Out; 2] {
    add2_with(a, b, sub, mode, fast_add)
}

/// `Fx - Fy` for both elements.
#[allow(dead_code)]
#[inline(always)]
pub(super) fn fast_sub2(a: [u64; 2], b: [u64; 2], mode: Mode) -> [Out; 2] {
    add2_with(a, b, true, mode, fast_add)
}

/// `Fx * Fy` for both elements, equal to `[fast_mul(a[0], b[0], ..), fast_mul(a[1], b[1], ..)]`.
#[inline(always)]
pub(super) fn fast_mul2(a: [u64; 2], b: [u64; 2], mode: Mode) -> [Out; 2] {
    mul2_with(a, b, mode, |x, y, m, _| fast_mul(x, y, m))
}

/// Pair add with an injectable scalar fallback (the tests replace it to prove the fallback runs
/// exactly for the lanes the vector path rejects).
#[inline(always)]
pub(super) fn add2_with(
    a: [u64; 2],
    b: [u64; 2],
    sub: bool,
    mode: Mode,
    fallback: impl Fn(u64, u64, bool, Mode) -> Out,
) -> [Out; 2] {
    finish(lanes::add(a, b, sub, mode), false, |i| {
        fallback(a[i], b[i], sub, mode)
    })
}

/// Pair multiply with an injectable scalar fallback (`false` is passed for the unused `sub`).
#[inline(always)]
pub(super) fn mul2_with(
    a: [u64; 2],
    b: [u64; 2],
    mode: Mode,
    fallback: impl Fn(u64, u64, Mode, bool) -> Out,
) -> [Out; 2] {
    finish(lanes::mul(a, b, mode), true, |i| {
        fallback(a[i], b[i], mode, false)
    })
}

/// The two results. A lane the vector path decided is a normal result (value and sign flags; a
/// product also sets MN) or, when `flag` is set for it, the zero form: AZ for a sum of two zeros,
/// a product with a zero operand sets MN only (AN stays clear). A rejected lane comes from
/// `fallback(lane)`.
#[inline(always)]
fn finish(l: lanes::Lanes, mul: bool, fallback: impl Fn(usize) -> Out) -> [Out; 2] {
    std::array::from_fn(|i| {
        if l.bad >> i & 1 != 0 {
            return fallback(i);
        }
        let neg = l.v[i] >> 39 == 1;
        let flag = l.flag >> i & 1 != 0;
        Out {
            v: l.v[i],
            an: neg && !(mul && flag),
            az: flag && !mul,
            mn: neg && mul,
            ..Out::default()
        }
    })
}

/// Scalar pair kernels: the target fallback and the cross-check of the vector path.
#[allow(dead_code)]
pub(super) mod scalar {
    use super::*;

    pub(in super::super) fn add2(a: [u64; 2], b: [u64; 2], sub: bool, mode: Mode) -> [Out; 2] {
        [0, 1].map(|i| fast_add(a[i], b[i], sub, mode))
    }

    pub(in super::super) fn mul2(a: [u64; 2], b: [u64; 2], mode: Mode) -> [Out; 2] {
        [0, 1].map(|i| fast_mul(a[i], b[i], mode))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::float;

    fn one() -> u64 {
        127 << 31
    }

    #[test]
    fn pair_kernels_equal_the_scalar_kernels_on_simple_operands() {
        let m = Mode::default();
        let a = [one() | 5 << 8, one() << 1 | 1 << 39];
        let b = [one() | 7 << 8, one() | 0x123 << 8];
        for sub in [false, true] {
            assert_eq!(fast_add2(a, b, sub, m), scalar::add2(a, b, sub, m));
        }
        assert_eq!(fast_sub2(a, b, m), scalar::add2(a, b, true, m));
        assert_eq!(fast_mul2(a, b, m), scalar::mul2(a, b, m));
    }

    #[test]
    fn one_lane_outside_the_normal_range_does_not_disturb_the_other() {
        let m = Mode::default();
        let normal = [one() | 9 << 8, one() | 3 << 8];
        for special in [0, 255 << 31, 1 << 39, (255 << 31) | 1 << 8, 1 << 8, 1 << 31] {
            for lane in 0..2 {
                let mut a = normal;
                a[lane] = special;
                let want = [0, 1].map(|i| float::add(a[i], normal[1 - i], false, false, m));
                let mut b = normal;
                b.swap(0, 1);
                assert_eq!(fast_add2(a, b, false, m), want, "add {a:#x?}");
                let want = [0, 1].map(|i| float::mul(a[i], b[i], m));
                assert_eq!(fast_mul2(a, b, m), want, "mul {a:#x?}");
            }
        }
    }

    #[test]
    fn rejected_lanes_use_the_fallback_and_accepted_lanes_do_not() {
        let m = Mode::default();
        // lane 0 normal, lane 1 infinity: only lane 1 may reach the fallback
        let a = [one(), 255 << 31];
        let b = [one(), one()];
        let marked = Out {
            v: 0xDEAD,
            ..Out::default()
        };
        let o = add2_with(a, b, false, m, |_, _, _, _| marked);
        assert_eq!(o[0], fast_add(one(), one(), false, m));
        assert_eq!(o[1], marked);
        let o = mul2_with(a, b, m, |_, _, _, _| marked);
        assert_eq!(o[0], fast_mul(one(), one(), m));
        assert_eq!(o[1], marked);
    }

    /// The vector path must decide ordinary lanes itself: a fallback taken for most lanes would
    /// still be bit-exact but would not be a fast path.
    #[test]
    fn ordinary_operands_rarely_reach_the_fallback() {
        use std::cell::Cell;
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut f = || (next() >> 24 & 0xFF_FFFF_FFFF & !(0xFF << 31)) | (100 + next() % 55) << 31;
        let (adds, muls, lanes) = (Cell::new(0), Cell::new(0), 100_000);
        for _ in 0..lanes / 2 {
            let (a, b) = ([f(), f()], [f(), f()]);
            for mode in [
                Mode::default(),
                Mode {
                    trunc: true,
                    ..Mode::default()
                },
            ] {
                add2_with(a, b, false, mode, |x, y, s, m| {
                    adds.set(adds.get() + 1);
                    fast_add(x, y, s, m)
                });
                mul2_with(a, b, mode, |x, y, m, _| {
                    muls.set(muls.get() + 1);
                    fast_mul(x, y, m)
                });
            }
        }
        // a TRUNC sum with a wide gap lands on a boundary now and then; nothing else should
        assert!(adds.get() < lanes / 4, "add fallbacks {}", adds.get());
        assert_eq!(muls.get(), 0, "mul fallbacks");
    }
}
