//! `COMP(Fx, Fy)` and the `Rx > Ry` history bit of the compare accumulator.
//!
//! OWNER: W1-KERN. Skeleton: forwards to `float_misc::compare` and `float_misc::greater`.
use super::{finish, FlagsOut, Mode, F40};
use crate::core::float_misc;

/// `COMP(a, b)`: only AZ (equal), AN (less) and AI (NaN) are set; the returned value is unused.
pub fn fcmp(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float_misc::compare(a, b, mode), st)
}

/// Whether `a > b` for the CACC history (a NaN operand counts as greater).
pub fn fgreater(a: F40, b: F40, mode: Mode) -> bool {
    float_misc::greater(a, b, mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::kernels::f40;

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
}
