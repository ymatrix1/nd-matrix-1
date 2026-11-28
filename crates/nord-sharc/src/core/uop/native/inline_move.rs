//! Inline emission of moves and memory (`FAM_MOVE`).
//!
//! OWNER: W5-NMOVE. Register moves, immediate loads and the SRAM window fast path (the window
//! check is inlined, every other address calls the handler, design 7.1). Each body equals its
//! Tier 1 handler on the fuzz. Skeleton: declines everything.
use super::emit::Emitter;
use super::FAM_MOVE;
use crate::core::uop::types::Uop;
use cranelift_frontend::FunctionBuilder;

/// The family bit that enables this emitter in `UopConfig::families`.
pub const FAMILY: u32 = FAM_MOVE;

/// Emitter of the inline move family.
pub(crate) struct MoveInline;

impl Emitter for MoveInline {
    fn try_emit(&mut self, u: &Uop, fb: &mut FunctionBuilder<'_>) -> bool {
        let _ = (u, fb);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::native::emit::emits_nothing;

    #[test]
    fn the_skeleton_emitter_declines_and_owns_the_move_bit() {
        assert!(emits_nothing(MoveInline));
        assert_eq!(FAMILY, 4);
    }
}
