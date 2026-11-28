//! Lowering of float instructions to uops.
//!
//! OWNER: W2-FLOAT. Floating-point compute (add, subtract, multiply, average, compare, select,
//! convert, abs/neg) on both elements. Fast uops call the exact kernels of `kernels/`; the
//! generic `FGEN` uop runs any other float compute through `exec::pe` (design 4.4).
//!
//! Skeleton: [`FloatFamily`] takes nothing, so every instruction stays an `Interp` uop (exact by
//! construction). The package replaces the body of `lower` and adds the matching handlers in
//! `exec/h_float.rs`.
use super::builder::BlockBuilder;
use super::{Family, LowerCtx};
use crate::core::decode_cache::CachedInsn;

/// The float family.
pub(in crate::core) struct FloatFamily;

impl Family for FloatFamily {
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
        assert_eq!(lower_first::<FloatFamily>(&[0x0001]), (false, 0));
        assert_eq!(
            lower_first::<FloatFamily>(&[0x013e, 0x0000, 0x1312]),
            (false, 0)
        );
    }
}
