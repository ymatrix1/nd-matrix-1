//! The emitter interface: how a uop becomes Cranelift IR.
//!
//! OWNER: W4-NCORE. An [`Emitter`] tries to emit a uop inline into a function under construction
//! and returns false to leave it call-threaded (a call of its Tier 1 handler), so a family that
//! emits nothing leaves the block exact. Skeleton: [`CallThreaded`] never emits inline, and
//! `emits_nothing` is the test helper the inline families use for their own skeleton test.
use crate::core::uop::types::Uop;
use cranelift_frontend::FunctionBuilder;

/// Inline emission of one uop.
pub(crate) trait Emitter {
    /// Emits `u` into `fb` and returns true, or returns false and emits nothing (the caller then
    /// emits a call of the Tier 1 handler).
    fn try_emit(&mut self, u: &Uop, fb: &mut FunctionBuilder<'_>) -> bool;
}

/// The emitter of call-threaded code: every uop stays a handler call.
pub(crate) struct CallThreaded;

impl Emitter for CallThreaded {
    fn try_emit(&mut self, u: &Uop, fb: &mut FunctionBuilder<'_>) -> bool {
        let _ = (u, fb);
        false
    }
}

/// Whether `e` declines a plain uop (test helper for the skeleton emitters).
#[cfg(test)]
pub(crate) fn emits_nothing(mut e: impl Emitter) -> bool {
    use crate::core::uop::types::UopKind;
    use cranelift_codegen::ir::Function;
    use cranelift_frontend::FunctionBuilderContext;
    let (mut func, mut ctx) = (Function::new(), FunctionBuilderContext::new());
    let mut fb = FunctionBuilder::new(&mut func, &mut ctx);
    !e.try_emit(&Uop::new(UopKind::NOP, 1, 0, 0, 0), &mut fb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_call_threaded_emitter_declines_every_uop() {
        assert!(emits_nothing(CallThreaded));
    }
}
