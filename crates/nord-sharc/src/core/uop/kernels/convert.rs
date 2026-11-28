//! Conversions: `FLOAT`, `FIX`/`TRUNC`, `RND` and `SCALB`.
//!
//! OWNER: W1-KERN. Skeleton: forwards to `float_conv::*`. Integer results come back as a 32-bit
//! value in the low bits of the [`F40`].
use super::{finish, FlagsOut, Mode, F40};
use crate::core::float_conv;

/// `Fn = FLOAT x BY scale`.
pub fn ffloat(x: i32, scale: i32, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float_conv::from_int(x, scale, mode), st)
}

/// `Rn = FIX a BY scale`, or `TRUNC` with `toward_zero`.
pub fn ffix(a: F40, scale: i32, toward_zero: bool, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float_conv::to_int(a, scale, toward_zero, mode), st)
}

/// `RND a`: round to a 32-bit boundary.
pub fn frnd(a: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float_conv::rnd(a, mode), st)
}

/// `SCALB a BY n`.
pub fn fscalb(a: F40, n: i32, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float_conv::scalb(a, n, mode), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::kernels::f40;

    #[test]
    fn conversions_round_trip_small_integers_and_scale_by_powers_of_two() {
        let (mut st, m) = (FlagsOut::default(), Mode::default());
        assert_eq!(ffloat(5, 0, m, &mut st), f40(5.0));
        assert_eq!(ffix(f40(5.0), 0, true, m, &mut st), 5);
        assert_eq!(ffix(f40(-5.5), 0, true, m, &mut st) as u32, (-5i32) as u32);
        assert_eq!(fscalb(f40(3.0), 2, m, &mut st), f40(12.0));
        assert_eq!(frnd(f40(1.25), m, &mut st), f40(1.25));
    }
}
