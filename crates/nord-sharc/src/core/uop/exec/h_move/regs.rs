//! Register-to-register transfers (type 5): `Ureg = Ureg` and the `Rr <-> Ss` swap.
//!
//! OWNER: W2-MOVE. Both run the interpreter's own helpers on the unconditional form (the condition
//! is TRUE, so both elements move): `capture_move` + `apply_move_if` carry the 40-bit float
//! extension, the PEy twin of a data register and the ASTAT / STKY pairs exactly as the census
//! measured them (`reg_moves.rs`), and `swap_regs` is the swap with both writes taken. The
//! operands are the register codes, so one handler serves every register class; system registers
//! are left to the interpreter by the lowering.
use super::layout::{REG_MOVE, REG_SWAP};
use crate::core::uop::types::{Exit, Handler, Uop, UopCtx};

const OK: u32 = Exit::Ok as u32;

/// `Ureg = Ureg`: `src` in bits 0..7, `dst` in bits 8..15 of `a`.
unsafe extern "C" fn reg_move(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let core = &mut *(*ctx).core;
    let a = (*u).a;
    let (src, dst) = ((a & 0x7F) as u8, (a >> 8 & 0x7F) as u8);
    let cap = core.capture_move(src);
    core.apply_move_if(dst, src, &cap, true, true);
    OK
}

/// `Rr <-> Ss`: `r` in bits 0..3, `s` in bits 4..7 of `a`.
unsafe extern "C" fn reg_swap(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let core = &mut *(*ctx).core;
    let a = (*u).a;
    core.swap_regs((a & 15) as usize, (a >> 4 & 15) as usize);
    OK
}

/// Fills the register-move kinds of `t`.
pub(super) fn register(t: &mut [Handler]) {
    t[usize::from(REG_MOVE.0)] = reg_move;
    t[usize::from(REG_SWAP.0)] = reg_swap;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::bail_handler;

    #[test]
    fn the_register_move_kinds_are_registered() {
        let marker: Handler = bail_handler;
        let mut t = vec![marker; 0x500];
        register(&mut t);
        let changed: Vec<usize> = (0..t.len())
            .filter(|&i| t[i] as usize != marker as usize)
            .collect();
        assert_eq!(changed, [0x328, 0x329]);
    }
}
