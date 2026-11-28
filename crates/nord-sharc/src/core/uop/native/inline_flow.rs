//! Inline emission of flow control (`FAM_FLOW`).
//!
//! OWNER: W5-NMOVE. Jumps, calls, returns, conditional skips and loop ends; a terminator writes
//! `next_pc` and exits exactly like its Tier 1 handler. Skeleton: declines everything.
use super::emit::Emitter;
use super::FAM_FLOW;
use crate::core::uop::types::Uop;
use cranelift_frontend::FunctionBuilder;

/// The family bit that enables this emitter in `UopConfig::families`.
pub const FAMILY: u32 = FAM_FLOW;

/// Emitter of the inline flow family.
pub(crate) struct FlowInline;

impl Emitter for FlowInline {
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
    fn the_skeleton_emitter_declines_and_owns_the_flow_bit() {
        assert!(emits_nothing(FlowInline));
        assert_eq!(FAMILY, 8);
    }
}
