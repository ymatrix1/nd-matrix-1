//! Lowering of flow instructions to uops.
//!
//! OWNER: W2-FLOW. Flow control (jump, call, return, delayed branches, DO loops, IDLE,
//! conditional skip). Loop and delay-slot state is materialized at every exit (design 4.3,
//! 4.5).
//!
//! Skeleton: [`FlowFamily`] takes nothing, so every instruction stays an `Interp` uop (exact by
//! construction). The package replaces the body of `lower` and adds the matching handlers in
//! `exec/h_flow.rs`.
use super::builder::BlockBuilder;
use super::{Family, LowerCtx};
use crate::core::decode_cache::CachedInsn;

/// The flow family.
pub(in crate::core) struct FlowFamily;

impl Family for FlowFamily {
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
        assert_eq!(lower_first::<FlowFamily>(&[0x0001]), (false, 0));
        assert_eq!(
            lower_first::<FlowFamily>(&[0x013e, 0x0000, 0x1312]),
            (false, 0)
        );
    }
}
