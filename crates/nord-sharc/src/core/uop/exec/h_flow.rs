//! Handlers of the flow range `0x400..0x500`.
//!
//! OWNER: W2-FLOW. Planned kinds: jump, conditional jump, call, return, delayed branches with two
//! slots (`UF_FIN` window, `Pending` set through the shared helper), `DO UNTIL` setup, loop-end
//! terminators (`FIN` runs `finish_step`), `Sys` (system-register read or write, then a deadline
//! re-check) and `IDLE` through the interpreter body. Terminators write `ctx.next_pc` and return
//! `Exit::Jump` (design 4.5).
//!
//! Skeleton: nothing registered, so a flow uop (none is emitted yet) would exit `Slow`.
use super::super::types::Handler;

/// Fills the flow range of `t`.
pub(crate) fn register(t: &mut [Handler]) {
    let _ = t;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::bail_handler;

    #[test]
    fn the_skeleton_registers_nothing() {
        let mut t = vec![bail_handler as Handler; 0x500];
        register(&mut t);
        assert!(t.iter().all(|h| *h as usize == bail_handler as usize));
    }
}
