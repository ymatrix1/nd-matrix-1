//! Sign and selection operations: `PASS`, `-`, `ABS` and `COPYSIGN`, `MIN`, `MAX`, `CLIP`.
//!
//! OWNER: W1-KERN. `kind` as in `float_misc`: sign 0..2, select 0..3 (a larger select kind is
//! `CLIP`, a larger sign kind negates nothing and sets no AS, like the reference). The reference
//! unpacks both operands into a `Un` with an `i128` ordering key and repacks the pick; here the
//! operands are `Opnd` (sign and flushed magnitude, `operand.rs`), the order is one `i64`
//! compare and the result is `sign << 39 | magnitude`.
//!
//! Semantics kept from the reference (all part of the grid): a denormal operand is a zero that
//! keeps its sign; a NaN operand gives all ones with AI only (no AN); AN is the sign of the
//! result even for a zero; `MIN(+0, -0)` is `-0` and `MAX(+0, -0)` is `+0`; RND32 clears the
//! extension of every operand first.
//!
//! Provenance: VERIFIED bit-identical to `float_misc::sign_op` and `float_misc::select_op`
//! (value and every flag) by `sign_equals_reference_on_the_class_grid` and
//! `select_equals_reference_on_the_class_grid_and_seeded_streams`.
use super::operand::{opnd, MAG39};
use super::{FlagsOut, Mode, F40};
use crate::core::float::ALL_ONES;

/// The NaN result of the sign and select operations: all ones, AI only.
/// VERIFIED: `float::Out::invalid` (simulator: AI only, no AN), checked by the grid tests.
#[inline(always)]
fn invalid(st: &mut FlagsOut) -> F40 {
    *st = FlagsOut {
        ai: true,
        ..FlagsOut::default()
    };
    ALL_ONES
}

/// `PASS` (0), negate (1) or `ABS` (2) of `a`.
#[inline]
pub fn fsign(a: F40, kind: u8, mode: Mode, st: &mut FlagsOut) -> F40 {
    let x = opnd(a, mode.r32);
    if x.nan() {
        return invalid(st);
    }
    let neg = match kind {
        0 => x.neg,
        1 => !x.neg,
        _ => false,
    };
    *st = FlagsOut {
        az: x.mag == 0,
        an: neg,
        as_: kind == 2 && x.neg,
        ..FlagsOut::default()
    };
    x.with_sign(neg)
}

/// `COPYSIGN` (0), `MIN` (1), `MAX` (2) or `CLIP` (3) of `a` and `b`.
#[inline]
pub fn fselect(a: F40, b: F40, kind: u8, mode: Mode, st: &mut FlagsOut) -> F40 {
    let (x, y) = (opnd(a, mode.r32), opnd(b, mode.r32));
    if x.nan() | y.nan() {
        return invalid(st);
    }
    let pick = match kind {
        0 => x.with_sign(y.neg),
        1 | 2 => {
            let (kx, ky) = (x.key(), y.key());
            // equal keys: MIN of +0 and -0 is -0 and MAX is +0 (any other tie is one value)
            let pick_x = if kx == ky {
                (kind == 1) == x.neg
            } else {
                (kx < ky) == (kind == 1)
            };
            if pick_x {
                x.with_sign(x.neg)
            } else {
                y.with_sign(y.neg)
            }
        }
        _ => {
            // |x| < |y| keeps x, else the magnitude of y with the sign of x
            let m = if x.mag < y.mag { x } else { y };
            m.with_sign(x.neg)
        }
    };
    *st = FlagsOut {
        az: pick & MAG39 == 0,
        an: pick >> 39 == 1,
        ..FlagsOut::default()
    };
    pick
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::float_misc;
    use crate::core::uop::kernels::f40;
    use crate::core::uop::kernels::grid::{self, Seen, AI, AN, AS, AZ};
    use crate::core::uop::verify::gen::Rng;

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

    fn check_sign(a: F40, kind: u8, mode: Mode, seen: &mut Seen) {
        let mut st = grid::dirty();
        let v = fsign(a, kind, mode, &mut st);
        let want = float_misc::sign_op(a, kind, mode);
        assert_eq!(st.out(v), want, "a={a:#x} kind={kind} {mode:?}");
        seen.result(&st);
        seen.operand(a);
    }

    fn check_select(a: F40, b: F40, kind: u8, mode: Mode, seen: &mut Seen) {
        let mut st = grid::dirty();
        let v = fselect(a, b, kind, mode, &mut st);
        let want = float_misc::select_op(a, b, kind, mode);
        assert_eq!(st.out(v), want, "a={a:#x} b={b:#x} kind={kind} {mode:?}");
        seen.result(&st);
        seen.operand(a);
        seen.operand(b);
    }

    #[test]
    fn sign_equals_reference_on_the_class_grid() {
        let mut seen = Seen::default();
        for mode in grid::modes() {
            for &a in &grid::grid() {
                // kinds 0..2 are the instruction forms; 3 and 4 pin the reference's fallthrough
                for kind in 0..5 {
                    check_sign(a, kind, mode, &mut seen);
                }
            }
        }
        let mut r = Rng::new(0x5160_0001);
        for mode in grid::modes() {
            for _ in 0..20_000 {
                check_sign(
                    grid::operand(&mut r),
                    (r.next_u64() % 3) as u8,
                    mode,
                    &mut seen,
                );
            }
        }
        seen.assert_reached(50_000, &[AZ, AN, AS, AI]);
    }

    #[test]
    fn select_equals_reference_on_the_class_grid_and_seeded_streams() {
        let mut seen = Seen::default();
        let g = grid::grid();
        for mode in grid::modes() {
            for &a in &g {
                for &b in &g {
                    // kind 4 pins that any select kind above 3 is CLIP, like the reference
                    for kind in 0..5 {
                        check_select(a, b, kind, mode, &mut seen);
                    }
                }
            }
        }
        let mut r = Rng::new(0x5160_0002);
        for mode in grid::modes() {
            for _ in 0..30_000 {
                let (a, b) = (grid::operand(&mut r), grid::operand(&mut r));
                let kind = (r.next_u64() % 4) as u8;
                check_select(a, b, kind, mode, &mut seen);
                // equal magnitudes with different extensions and signs are the tie cases
                check_select(a, a ^ (1 << 39), kind, mode, &mut seen);
                check_select(a, grid::near(&mut r, a), kind, mode, &mut seen);
            }
        }
        seen.assert_reached(800_000, &[AZ, AN, AI]);
    }

    #[test]
    fn min_and_max_of_signed_zeros_and_equal_values_follow_the_reference() {
        let (mut st, m) = (grid::dirty(), Mode::default());
        let (pz, nz) = (0u64, 1u64 << 39);
        assert_eq!(fselect(pz, nz, 1, m, &mut st), nz, "MIN(+0, -0)");
        assert_eq!(fselect(nz, pz, 1, m, &mut st), nz);
        assert_eq!(fselect(pz, nz, 2, m, &mut st), pz, "MAX(+0, -0)");
        assert_eq!(fselect(nz, pz, 2, m, &mut st), pz);
        assert!(st.az && !st.an);
        // a denormal is a zero with a sign
        let nd = 1u64 << 39 | 7 << 8;
        assert_eq!(fselect(nd, pz, 2, m, &mut st), pz);
        assert_eq!(fselect(nd, nd, 1, m, &mut st), nz);
    }
}
