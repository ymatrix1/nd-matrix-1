//! `Fx + Fy` and `Fx - Fy`.
//!
//! OWNER: W1-KERN. Skeleton: forwards to `float::add`. Replacement rule: equal to the forwarded
//! call on every operand class pair (zero, denormal, NaN, infinity, tag and low byte), both
//! rounding modes, RND32 on and off, plus seeded random.
use super::{finish, FlagsOut, Mode, F40};
use crate::core::float;

/// `a + b`.
pub fn fadd(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float::add(a, b, false, false, mode), st)
}

/// `a - b`.
pub fn fsub(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float::add(a, b, true, false, mode), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::kernels::f40;

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
}
