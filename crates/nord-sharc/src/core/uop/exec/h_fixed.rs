//! Handlers of the fixed-point range `0x200..0x300`.
//!
//! OWNER: W2-FIXED. Planned kinds: `XGEN` (generic, through `Pe::exec`), `XADDSUB`, `XLOGIC`,
//! `XSHIFT`, `XMUL`, `XCMP`, with saturation and rounding modes of MODE1 as block guards and MR
//! spills exact (design 7.1, 4.5).
//!
//! Skeleton: nothing registered, so a fixed-point uop (none is emitted yet) would exit `Slow`.
use super::super::types::Handler;

/// Fills the fixed-point range of `t`.
pub(crate) fn register(t: &mut [Handler]) {
    let _ = t;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::bail_handler;

    #[test]
    fn the_skeleton_registers_nothing() {
        let marker: Handler = bail_handler;
        let mut t = vec![marker; 0x500];
        register(&mut t);
        assert!(t.iter().all(|h| *h as usize == marker as usize));
    }
}
