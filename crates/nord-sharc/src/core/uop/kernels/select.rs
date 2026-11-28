//! Sign and selection operations: `PASS`, `-`, `ABS` and `COPYSIGN`, `MIN`, `MAX`, `CLIP`.
//!
//! OWNER: W1-KERN. Skeleton: forwards to `float::sign_op` and `float::select_op` (`kind` as
//! there: sign 0..2, select 0..3). Same replacement rule as `add.rs`.
use super::{finish, FlagsOut, Mode, F40};
use crate::core::float_misc;

/// `PASS` (0), negate (1) or `ABS` (2) of `a`.
pub fn fsign(a: F40, kind: u8, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float_misc::sign_op(a, kind, mode), st)
}

/// `COPYSIGN` (0), `MIN` (1), `MAX` (2) or `CLIP` (3) of `a` and `b`.
pub fn fselect(a: F40, b: F40, kind: u8, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(float_misc::select_op(a, b, kind, mode), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::kernels::f40;

    #[test]
    fn sign_and_selection_kinds_pick_the_documented_operand() {
        let (mut st, m) = (FlagsOut::default(), Mode::default());
        assert_eq!(fsign(f40(-2.0), 2, m, &mut st), f40(2.0));
        assert!(st.as_, "ABS of a negative sets AS");
        assert_eq!(fsign(f40(2.0), 1, m, &mut st), f40(-2.0));
        assert_eq!(fselect(f40(1.0), f40(3.0), 1, m, &mut st), f40(1.0));
        assert_eq!(fselect(f40(1.0), f40(3.0), 2, m, &mut st), f40(3.0));
        assert_eq!(fselect(f40(1.0), f40(-3.0), 0, m, &mut st), f40(-1.0));
    }
}
