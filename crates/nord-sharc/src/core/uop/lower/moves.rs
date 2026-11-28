//! Lowering of move instructions to uops.
//!
//! OWNER: W2-MOVE. Register and memory moves (DAG pre/post-modify, circular buffers, bit-
//! reversed addressing, DM/PM access, universal register moves). Every handler checks the
//! window and the circular-buffer preconditions before it mutates and exits `Slow` otherwise
//! (design 4.4, 7.2).
//!
//! Skeleton: [`MoveFamily`] takes nothing, so every instruction stays an `Interp` uop (exact by
//! construction). The package replaces the body of `lower` and adds the matching handlers in
//! `exec/h_move.rs`.
use super::builder::BlockBuilder;
use super::{Family, LowerCtx};
use crate::core::decode_cache::CachedInsn;

/// The move family.
pub(in crate::core) struct MoveFamily;

impl Family for MoveFamily {
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
        assert_eq!(lower_first::<MoveFamily>(&[0x0001]), (false, 0));
        assert_eq!(
            lower_first::<MoveFamily>(&[0x013e, 0x0000, 0x1312]),
            (false, 0)
        );
    }
}
