//! Call-threaded emission: a block as a straight line of handler calls.
//!
//! OWNER: W4-NCORE. Each uop becomes `h(ctx, &uop)` with the handler address and the uop address
//! as constants, followed by the exit check of design 4.5. This is the K0 probe of the plan (gate
//! K0: at most 70 cycles per step) and the fallback for every uop an inline family declines.
//! Skeleton: only the address helper the emitter needs.
use crate::core::uop::types::Handler;

/// The code address of a handler, as the constant of an indirect call.
pub fn handler_addr(h: Handler) -> usize {
    h as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::bail_handler;

    #[test]
    fn a_handler_has_a_stable_non_null_address() {
        let a = handler_addr(bail_handler);
        assert_ne!(a, 0);
        assert_eq!(a, handler_addr(bail_handler));
    }
}
