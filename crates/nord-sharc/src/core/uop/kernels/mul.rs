//! `Fx * Fy` of the multiplier.
//!
//! OWNER: W1-KERN. The scalar form is `aot_fast::fast_mul` (32 x 32 significand product in one
//! `u64` multiply, one rounding in `round_norm`, a cold path for zero, denormal, infinity and
//! NaN); the paired form is `aot_simd::fast_mul2` (`pmuludq`, PEx in lane 0, PEy in lane 1). The
//! multiplier flags `mn`, `mv`, `mu`, `mi` are set; the ALU flags of the result are not used.
//!
//! Provenance: VERIFIED bit-identical to `float::mul` (value and every flag) by
//! `mul_equals_reference_on_the_class_grid_and_seeded_streams` and
//! `paired_mul_equals_scalar_calls_and_the_reference`; the wrapped kernels carry their own sweeps
//! (`aot_fast.rs`, `aot_simd_sweep.rs`).
use super::{finish, finish2, FlagsOut, Mode, F40};
use crate::core::aot_fast::fast_mul;
use crate::core::aot_simd::fast_mul2;

/// `a * b`.
#[inline(always)]
pub fn fmul(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(fast_mul(a, b, mode), st)
}

/// `a * b` on both elements: `[PEx, PEy]`.
#[inline(always)]
pub fn fmul2(a: [F40; 2], b: [F40; 2], mode: Mode, st: &mut [FlagsOut; 2]) -> [F40; 2] {
    finish2(fast_mul2(a, b, mode), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::float;
    use crate::core::uop::kernels::f40;
    use crate::core::uop::kernels::grid::{self, Seen, AZ, MI, MN, MU, MV};
    use crate::core::uop::verify::gen::Rng;

    fn check(a: F40, b: F40, mode: Mode, seen: &mut Seen) {
        let mut st = grid::dirty();
        let v = fmul(a, b, mode, &mut st);
        assert_eq!(
            st.out(v),
            float::mul(a, b, mode),
            "a={a:#x} b={b:#x} {mode:?}"
        );
        seen.result(&st);
        seen.operand(a);
        seen.operand(b);
    }

    #[test]
    fn mul_follows_ieee_single_and_sets_the_negative_flag() {
        let (mut st, m) = (FlagsOut::default(), Mode::default());
        assert_eq!(fmul(f40(1.5), f40(-2.0), m, &mut st), f40(-3.0));
        assert!(st.mn);
        assert_eq!(fmul(f40(0.5), f40(0.5), m, &mut st), f40(0.25));
        assert!(!st.mn && !st.mv);
    }

    #[test]
    fn mul_equals_reference_on_the_class_grid_and_seeded_streams() {
        let mut seen = Seen::default();
        let g = grid::grid();
        for mode in grid::modes() {
            for &a in &g {
                for &b in &g {
                    check(a, b, mode, &mut seen);
                }
            }
        }
        let mut r = Rng::new(0x3070_0001);
        for mode in grid::modes() {
            for _ in 0..60_000 {
                let (a, b) = (grid::operand(&mut r), grid::operand(&mut r));
                check(a, b, mode, &mut seen);
            }
        }
        // overflow (MV), flushed underflow (MU), invalid (MI), negative (MN) and zero all occur
        seen.assert_reached(500_000, &[AZ, MN, MV, MU, MI]);
    }

    #[test]
    fn paired_mul_equals_scalar_calls_and_the_reference() {
        let mut seen = Seen::default();
        let g = grid::grid();
        let mut r = Rng::new(0x3070_0002);
        for mode in grid::modes() {
            for (i, &a0) in g.iter().enumerate() {
                for &b0 in &g {
                    let (a1, b1) = match i % 3 {
                        0 => (grid::operand(&mut r), grid::operand(&mut r)),
                        1 => (grid::near(&mut r, a0), b0),
                        _ => (b0, a0),
                    };
                    let (a, b) = ([a0, a1], [b0, b1]);
                    let mut st = [grid::dirty(); 2];
                    let v = fmul2(a, b, mode, &mut st);
                    for lane in 0..2 {
                        let mut one = grid::dirty();
                        let s = fmul(a[lane], b[lane], mode, &mut one);
                        assert_eq!((v[lane], st[lane]), (s, one), "lane {lane} {a:#x?} {b:#x?}");
                        assert_eq!(st[lane].out(v[lane]), float::mul(a[lane], b[lane], mode));
                        seen.result(&st[lane]);
                    }
                    seen.operand(a0);
                    seen.operand(a1);
                }
            }
        }
        seen.assert_reached(300_000, &[AZ, MN, MV, MU, MI]);
    }
}
