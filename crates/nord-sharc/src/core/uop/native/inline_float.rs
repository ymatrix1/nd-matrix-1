//! Inline emission of the float family (`FAM_FLOAT`).
//!
//! OWNER: W5-NFLOAT. Add, subtract, multiply, average, select, compare and convert, each with a
//! static instruction-count ceiling (gates K1 and K2) and each equal to its Tier 1 handler on the
//! fuzz through `exact_diff`. Skeleton: declines everything, so float uops stay call-threaded.
use super::emit::Emitter;
use super::FAM_FLOAT;
use crate::core::uop::types::Uop;
use cranelift_frontend::FunctionBuilder;

/// The family bit that enables this emitter in `UopConfig::families`.
pub const FAMILY: u32 = FAM_FLOAT;

/// Emitter of the inline float family.
pub(crate) struct FloatInline;

impl Emitter for FloatInline {
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
    fn the_skeleton_emitter_declines_and_owns_the_float_bit() {
        assert!(emits_nothing(FloatInline));
        assert_eq!(FAMILY, 2);
    }
}
