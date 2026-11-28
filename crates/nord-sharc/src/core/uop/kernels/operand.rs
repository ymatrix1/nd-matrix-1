//! A float operand reduced to what the sign, select, compare and convert kernels need: its sign
//! and its flushed magnitude. One integer compare then replaces the `unpack` / `key` / `repack`
//! round trip of `float_misc.rs` (a `Un` with an `i128` key per operand).
//!
//! OWNER: W1-KERN. Layout (VERIFIED: `float.rs` module doc and
//! `operand_matches_unpack_on_the_class_grid_and_in_both_extension_modes`):
//! bit 39 sign, bits 38..31 biased exponent, bits 30..0 mantissa, the low 8 bits being the
//! extension that RND32 clears. The magnitude is the low 39 bits. Order of magnitudes is the
//! numeric order: zero (exponent 0, a denormal is flushed to zero), normals by (exponent, mantissa),
//! then infinity (exponent 255, mantissa 0), then NaN (exponent 255, mantissa non-zero).
use super::F40;

/// The 39 magnitude bits of a 40-bit float.
/// VERIFIED: `float.rs` layout, checked by `operand_matches_unpack_on_the_class_grid_and_in_both_extension_modes` on the class grid.
pub(super) const MAG39: u64 = (1 << 39) - 1;
/// Magnitude of infinity: exponent 255, mantissa 0. A larger magnitude is a NaN.
/// VERIFIED: `float::unpack` class `Inf`, checked by `operand_matches_unpack_on_the_class_grid_and_in_both_extension_modes`.
pub(super) const INF_MAG: u64 = 0xFF << 31;
/// Smallest magnitude with a non-zero exponent field; below it the operand is zero or denormal.
/// VERIFIED: `float::unpack` class `Zero` (exponent 0), checked by `operand_matches_unpack_on_the_class_grid_and_in_both_extension_modes`.
const NORMAL_MIN: u64 = 1 << 31;

/// Sign and magnitude of one operand after the RND32 mask and the denormal flush.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Opnd {
    pub neg: bool,
    /// Flushed magnitude: 0 for zero and denormals, else the low 39 bits (infinity is
    /// [`INF_MAG`], a NaN is above it).
    pub mag: u64,
}

/// Decodes `v`; with `r32` the eight extension bits are cleared first (like `float::unpack`).
#[inline(always)]
pub(super) fn opnd(v: F40, r32: bool) -> Opnd {
    let m = v & !(0xFFu64 * u64::from(r32));
    let mag = m & MAG39;
    Opnd {
        neg: (m >> 39) & 1 == 1,
        mag: if mag < NORMAL_MIN { 0 } else { mag },
    }
}

impl Opnd {
    /// Whether the operand is a NaN.
    #[inline(always)]
    pub fn nan(self) -> bool {
        self.mag > INF_MAG
    }

    /// Signed magnitude: orders operands numerically, both zeros are equal (`float_misc::key`).
    #[inline(always)]
    pub fn key(self) -> i64 {
        let m = self.mag as i64;
        if self.neg {
            -m
        } else {
            m
        }
    }

    /// The register image with sign `neg` and this magnitude (what `float::repack` builds for a
    /// zero, a normal or an infinity).
    #[inline(always)]
    pub fn with_sign(self, neg: bool) -> F40 {
        u64::from(neg) << 39 | self.mag
    }
}

#[cfg(test)]
mod tests {
    use super::super::grid::{class_of, grid, Class};
    use super::*;
    use crate::core::float::{repack, unpack, Un};

    #[test]
    fn operand_matches_unpack_on_the_class_grid_and_in_both_extension_modes() {
        let mut seen = [0usize; 6];
        for v in grid() {
            for r32 in [false, true] {
                let (x, u) = (opnd(v, r32), unpack(v, r32));
                assert_eq!(x.nan(), u.class == crate::core::float::Class::Nan, "{v:#x}");
                if x.nan() {
                    continue;
                }
                assert_eq!(x.neg, u.neg, "{v:#x} r32 {r32}");
                // repack of a zero, normal or infinity is exactly sign and flushed magnitude
                assert_eq!(x.with_sign(u.neg), repack(u), "{v:#x} r32 {r32}");
                let other = Un { neg: !u.neg, ..u };
                assert_eq!(x.with_sign(!u.neg), repack(other), "{v:#x} r32 {r32}");
            }
            seen[class_of(v) as usize] += 1;
        }
        assert!(
            seen.iter().all(|&n| n > 0),
            "every class is on the grid: {seen:?}"
        );
    }

    #[test]
    fn key_orders_numerically_and_treats_both_zeros_alike() {
        let one = opnd(127 << 31, false);
        let two = opnd(128 << 31, false);
        let neg_two = opnd(1 << 39 | 128 << 31, false);
        let (pz, nz) = (opnd(0, false), opnd(1 << 39, false));
        let denorm = opnd(1 << 39 | 5 << 8, false);
        let inf = opnd(INF_MAG, false);
        assert!(neg_two.key() < one.key() && one.key() < two.key() && two.key() < inf.key());
        assert_eq!((pz.key(), nz.key(), denorm.key()), (0, 0, 0));
        assert!(
            denorm.neg && denorm.mag == 0,
            "a flushed denormal keeps its sign"
        );
        assert!(opnd(INF_MAG | 1, false).nan() && !inf.nan());
        assert_eq!(
            opnd(127 << 31 | 0xAB, true),
            opnd(127 << 31, true),
            "RND32 drops the extension"
        );
    }
}
