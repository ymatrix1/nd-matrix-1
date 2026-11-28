//! OWNER: W1-XALU. Executor arms of the data-processing uops (skeleton: no arm yet; the
//! lowerers emit `GENERIC` until a family lands, so nothing reaches here).
//!
//! Spec (design note 4.3, WP W1-XALU): one arm per `(OP, S, FORM)` const-generic combination over
//! the register slice and the flag nibble `f` (`n << 3 | z << 2 | c << 1 | v`, I4): the shifter
//! (`alu::shift` semantics, amounts 0/32/33/255/256 of a register shift, RRX), `alu::add_c` for
//! add/sub/rsb/adc/sbc/rsc/cmp/cmn (`adc`/`sbc`/`rsc` take the pre-instruction C), logic ops keep V
//! and take the shifter carry, the specialised ids `MOV_IMM..=ORR_IMM` equal the generic id they
//! replace, `r15` is never touched (I2). Semantics are those of `Cpu::dp_exec`; the tests compare
//! every arm against it on random states and edge values.
//!
//! Registered statically in `exec::run_sb` (family `k::Family::Dp`). The signature is frozen.

use super::uop::Uop;

/// Execute one data-processing uop on the registers `r` and the flag nibble `f`.
pub fn exec_dp(r: &mut [u32; 16], f: &mut u8, u: &Uop) {
    let _ = (r, f);
    unreachable!(
        "uop kind {} has no executor arm yet (OWNER W1-XALU)",
        u.kind
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::uop::k;

    #[test]
    #[should_panic(expected = "W1-XALU")]
    fn the_skeleton_has_no_arm_and_says_who_owns_it() {
        exec_dp(&mut [0; 16], &mut 0, &Uop::new(k::MOV_IMM, 0x1000));
    }
}
