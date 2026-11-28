//! Handlers of the float range `0x100..0x200`.
//!
//! OWNER: W2-FLOAT. Planned kinds (constants are added here, in this range only): `FGEN`
//! (generic, through `exec::pe` / `Pe::exec`), `FADD`, `FSUB`, `FMUL`, `FAVG`, `FCMP`, `FSEL`,
//! `FCONV`, `FABSNEG`. Handlers run both elements in place (PEx, and PEy when the uop has
//! `UF_SIMD`), use the exact kernels of `kernels/`, check every precondition before they mutate
//! and never panic (design 4.4, 4.5).
//!
//! Skeleton: nothing registered, so a float uop (none is emitted yet) would exit `Slow`.
use super::super::types::Handler;

/// Fills the float range of `t`.
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
