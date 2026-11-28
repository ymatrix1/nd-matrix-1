//! `Fx * Fy` of the multiplier.
//!
//! OWNER: W1-KERN. Skeleton: forwards to `float::mul`, which sets the multiplier flags
//! (`mn`, `mv`, `mu`, `mi`). Same replacement rule as `add.rs`.
use super::{finish, FlagsOut, Mode, F40};
use crate::core::float;

/// `a * b`.
pub fn fmul(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float::mul(a, b, mode), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::kernels::f40;

    #[test]
    fn mul_follows_ieee_single_and_sets_the_negative_flag() {
        let (mut st, m) = (FlagsOut::default(), Mode::default());
        assert_eq!(fmul(f40(1.5), f40(-2.0), m, &mut st), f40(-3.0));
        assert!(st.mn);
        assert_eq!(fmul(f40(0.5), f40(0.5), m, &mut st), f40(0.25));
        assert!(!st.mn && !st.mv);
    }
}
