//! `(Fx + Fy) / 2`: the exponent is decremented before rounding, so the result differs from
//! add-then-halve at the edges of the range (the 0x89 reference instruction, design 4.4).
//!
//! OWNER: W1-KERN. Skeleton: forwards to `float::add` with `halve`. The package covers the
//! overflow and underflow edges with the exhaustive class grid.
use super::{finish, FlagsOut, Mode, F40};
use crate::core::float;

/// `(a + b) / 2`.
pub fn favg(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float::add(a, b, false, true, mode), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::kernels::f40;

    #[test]
    fn average_halves_the_sum_without_losing_range() {
        let (mut st, m) = (FlagsOut::default(), Mode::default());
        assert_eq!(favg(f40(3.0), f40(1.0), m, &mut st), f40(2.0));
        // the sum of the two largest singles overflows, the average does not
        let big = f40(f32::MAX);
        assert_eq!(favg(big, big, m, &mut st), big);
        assert!(!st.av);
    }
}
