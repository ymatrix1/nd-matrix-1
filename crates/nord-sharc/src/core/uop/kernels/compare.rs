//! `COMP(Fx, Fy)` and the `Rx > Ry` history bit of the compare accumulator.
//!
//! OWNER: W1-KERN. One `i64` compare of the signed flushed magnitudes (`operand.rs`) replaces the
//! two `unpack` calls and the `i128` keys of `float_misc::compare` and `float_misc::greater`.
//! Semantics kept from the reference: AZ when the operands are equal (both zeros are equal, a
//! denormal is a zero), AN when `a < b`, AI alone when either is a NaN (the result then reads as
//! "greater" for the CACC history), RND32 clears the extensions first.
//!
//! Provenance: VERIFIED bit-identical to `float_misc::compare` and `float_misc::greater` by
//! `compare_equals_reference_on_the_class_grid_and_seeded_streams`.
use super::operand::opnd;
use super::{FlagsOut, Mode, F40};

/// `COMP(a, b)`: only AZ (equal), AN (less) and AI (NaN) are set; the returned value is unused.
#[inline]
pub fn fcmp(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    let (x, y) = (opnd(a, mode.r32), opnd(b, mode.r32));
    let (kx, ky) = (x.key(), y.key());
    *st = if x.nan() | y.nan() {
        FlagsOut {
            ai: true,
            ..FlagsOut::default()
        }
    } else {
        FlagsOut {
            az: kx == ky,
            an: kx < ky,
            ..FlagsOut::default()
        }
    };
    0
}

/// Whether `a > b` for the CACC history (a NaN operand counts as greater).
#[inline]
pub fn fgreater(a: F40, b: F40, mode: Mode) -> bool {
    let (x, y) = (opnd(a, mode.r32), opnd(b, mode.r32));
    x.nan() | y.nan() | (x.key() > y.key())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::float_misc;
    use crate::core::uop::kernels::f40;
    use crate::core::uop::kernels::grid::{self, Seen, AI, AN, AZ};
    use crate::core::uop::verify::gen::Rng;

    fn check(a: F40, b: F40, mode: Mode, seen: &mut Seen) {
        let mut st = grid::dirty();
        let v = fcmp(a, b, mode, &mut st);
        assert_eq!(
            st.out(v),
            float_misc::compare(a, b, mode),
            "a={a:#x} b={b:#x} {mode:?}"
        );
        assert_eq!(
            fgreater(a, b, mode),
            float_misc::greater(a, b, mode),
            "greater a={a:#x} b={b:#x} {mode:?}"
        );
        seen.result(&st);
        seen.operand(a);
        seen.operand(b);
    }

    #[test]
    fn compare_sets_equal_and_less_and_greater_follows_the_order() {
        let (mut st, m) = (FlagsOut::default(), Mode::default());
        fcmp(f40(1.0), f40(1.0), m, &mut st);
        assert!(st.az && !st.an);
        fcmp(f40(1.0), f40(2.0), m, &mut st);
        assert!(!st.az && st.an);
        fcmp(f40(f32::NAN), f40(2.0), m, &mut st);
        assert!(st.ai);
        assert!(fgreater(f40(2.0), f40(1.0), m));
        assert!(!fgreater(f40(1.0), f40(2.0), m));
        assert!(fgreater(f40(f32::NAN), f40(2.0), m));
    }

    #[test]
    fn compare_equals_reference_on_the_class_grid_and_seeded_streams() {
        let mut seen = Seen::default();
        let g = grid::grid();
        for mode in grid::modes() {
            for &a in &g {
                for &b in &g {
                    check(a, b, mode, &mut seen);
                }
            }
        }
        let mut r = Rng::new(0xC0A9_0001);
        for mode in grid::modes() {
            for _ in 0..40_000 {
                let (a, b) = (grid::operand(&mut r), grid::operand(&mut r));
                check(a, b, mode, &mut seen);
                check(a, grid::near(&mut r, a), mode, &mut seen);
                // equal values, and equal values of opposite sign (zeros compare equal)
                check(a, a, mode, &mut seen);
                check(a, a ^ (1 << 39), mode, &mut seen);
            }
        }
        seen.assert_reached(400_000, &[AZ, AN, AI]);
        // the three verdicts all happened, so an always-equal or always-less kernel fails
        assert!(seen.flags[AZ] > 1000 && seen.flags[AN] > 1000 && seen.flags[AI] > 1000);
    }
}
