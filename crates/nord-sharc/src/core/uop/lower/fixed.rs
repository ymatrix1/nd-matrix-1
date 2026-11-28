//! Lowering of fixed-point instructions to uops.
//!
//! OWNER: W2-FIXED. Fixed-point compute (ALU, multiplier, shifter, MR registers) and
//! multifunction instructions. Each fast uop reproduces flags, hidden state (MRF/MRB, SIMD
//! element) and saturation exactly (design 4.4).
//!
//! Skeleton: [`FixedFamily`] takes nothing, so every instruction stays an `Interp` uop (exact by
//! construction). The package replaces the body of `lower` and adds the matching handlers in
//! `exec/h_fixed.rs`.
use super::builder::BlockBuilder;
use super::{Family, LowerCtx};
use crate::core::decode_cache::CachedInsn;

/// The fixed-point family.
pub(in crate::core) struct FixedFamily;

impl Family for FixedFamily {
    fn lower(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) -> bool {
        let _ = (cx, c, out);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::lower::lower_first;

    #[test]
    fn the_skeleton_takes_no_instruction() {
        // NOP and R3 = R1 + R2
        assert_eq!(lower_first::<FixedFamily>(&[0x0001]), (false, 0));
        assert_eq!(
            lower_first::<FixedFamily>(&[0x013e, 0x0000, 0x1312]),
            (false, 0)
        );
    }
}
