//! `Fx + Fy` and `Fx - Fy`.
//!
//! OWNER: W1-KERN. The scalar forms are `aot_fast::fast_add` (the f64 add trick: one hardware
//! add and one integer rounding for ordinary operands, a cold path for zero, denormal, infinity
//! and NaN, the exact integer kernel for sums that sit on a rounding boundary). The paired forms
//! are `aot_simd::fast_add2` and `fast_sub2`: PEx in lane 0, PEy in lane 1, one SSE2 vector, each
//! lane the vector path cannot decide recomputed by the scalar kernel on its own operands.
//!
//! Provenance: VERIFIED bit-identical to `float::add` (value and every flag) by
//! `add_and_sub_equal_reference_on_the_class_grid_and_seeded_streams` and
//! `paired_forms_equal_scalar_calls_and_the_reference`; the wrapped kernels carry their own
//! sweeps (`aot_fast.rs`, `aot_simd_sweep.rs`) and the f64 add argument of `aot_fadd.rs`.
use super::{finish, finish2, FlagsOut, Mode, F40};
use crate::core::aot_fast::fast_add;
use crate::core::aot_simd::{fast_add2, fast_sub2};

/// `a + b`.
#[inline(always)]
pub fn fadd(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(fast_add(a, b, false, mode), st)
}

/// `a - b`.
#[inline(always)]
pub fn fsub(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(fast_add(a, b, true, mode), st)
}

/// `a + b` on both elements: `[PEx, PEy]`.
#[inline(always)]
pub fn fadd2(a: [F40; 2], b: [F40; 2], mode: Mode, st: &mut [FlagsOut; 2]) -> [F40; 2] {
    finish2(fast_add2(a, b, false, mode), st)
}

/// `a - b` on both elements: `[PEx, PEy]`.
#[inline(always)]
pub fn fsub2(a: [F40; 2], b: [F40; 2], mode: Mode, st: &mut [FlagsOut; 2]) -> [F40; 2] {
    finish2(fast_sub2(a, b, mode), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::float;
    use crate::core::uop::kernels::f40;
    use crate::core::uop::kernels::grid::{self, Seen, AI, AN, AU, AV, AZ};
    use crate::core::uop::verify::gen::Rng;

    /// Both kernels on one operand pair against `float::add`, with a dirty flag word so a kernel
    /// that leaves a flag alone is caught.
    fn check(a: F40, b: F40, mode: Mode, seen: &mut Seen) {
        for sub in [false, true] {
            let mut st = grid::dirty();
            let v = if sub {
                fsub(a, b, mode, &mut st)
            } else {
                fadd(a, b, mode, &mut st)
            };
            let want = float::add(a, b, sub, false, mode);
            assert_eq!(st.out(v), want, "a={a:#x} b={b:#x} sub={sub} {mode:?}");
            seen.result(&st);
        }
        seen.operand(a);
        seen.operand(b);
    }

    #[test]
    fn add_and_sub_follow_ieee_single_on_simple_values() {
        let (mut st, m) = (FlagsOut::default(), Mode::default());
        assert_eq!(fadd(f40(1.5), f40(2.0), m, &mut st), f40(3.5));
        assert!(!st.az && !st.an);
        assert_eq!(fsub(f40(1.5), f40(2.0), m, &mut st), f40(-0.5));
        assert!(st.an && !st.az);
        assert_eq!(fsub(f40(2.0), f40(2.0), m, &mut st), 0);
        assert!(st.az);
    }

    #[test]
    fn add_and_sub_equal_reference_on_the_class_grid_and_seeded_streams() {
        let mut seen = Seen::default();
        let g = grid::grid();
        for mode in grid::modes() {
            for &a in &g {
                for &b in &g {
                    check(a, b, mode, &mut seen);
                }
            }
        }
        let mut r = Rng::new(0xADD0_0001);
        for mode in grid::modes() {
            for _ in 0..30_000 {
                let (a, b) = (grid::operand(&mut r), grid::operand(&mut r));
                check(a, b, mode, &mut seen);
                check(a, grid::near(&mut r, a), mode, &mut seen);
            }
            // every exponent gap, so alignment, sticky bits and cancellation all occur
            for gap in 0..70u64 {
                for _ in 0..300 {
                    let a = grid::operand(&mut r) & !(0xFFu64 << 31) | (150 + gap.min(100)) << 31;
                    let b = grid::operand(&mut r) & !(0xFFu64 << 31) | 150 << 31;
                    check(a, b, mode, &mut seen);
                }
            }
        }
        // 144 x 144 x 8 x 2 grid cases alone, plus the streams
        seen.assert_reached(400_000, &[AZ, AN, AV, AI, AU]);
    }

    #[test]
    fn paired_forms_equal_scalar_calls_and_the_reference() {
        let mut seen = Seen::default();
        let g = grid::grid();
        let mut r = Rng::new(0xADD0_0002);
        for mode in grid::modes() {
            for (i, &a0) in g.iter().enumerate() {
                for &b0 in &g {
                    // the other lane is ordinary, near, or the mirror of this one
                    let (a1, b1) = match i % 3 {
                        0 => (grid::operand(&mut r), grid::operand(&mut r)),
                        1 => (grid::near(&mut r, a0), b0),
                        _ => (b0, a0),
                    };
                    let (a, b) = ([a0, a1], [b0, b1]);
                    for sub in [false, true] {
                        let mut st = [grid::dirty(); 2];
                        let v = if sub {
                            fsub2(a, b, mode, &mut st)
                        } else {
                            fadd2(a, b, mode, &mut st)
                        };
                        for lane in 0..2 {
                            let mut one = grid::dirty();
                            let s = if sub {
                                fsub(a[lane], b[lane], mode, &mut one)
                            } else {
                                fadd(a[lane], b[lane], mode, &mut one)
                            };
                            assert_eq!(
                                (v[lane], st[lane]),
                                (s, one),
                                "lane {lane} {a:#x?} {b:#x?}"
                            );
                            let want = float::add(a[lane], b[lane], sub, false, mode);
                            assert_eq!(st[lane].out(v[lane]), want, "lane {lane} sub {sub}");
                            seen.result(&st[lane]);
                        }
                    }
                    seen.operand(a1);
                    seen.operand(a0);
                }
            }
        }
        seen.assert_reached(150_000, &[AZ, AN, AV, AI, AU]);
    }
}
