//! OWNER: W1-XMEM. Executor arms of the memory uops (skeleton: no arm yet; the lowerers emit
//! `GENERIC` until a family lands, so nothing reaches here).
//!
//! Spec (design note 4.3, WP W1-XMEM): arms generic over [`SbBus`]: word load rotated by
//! `8 * (a & 3)`, store at `a & !3`, halfword at `a & !1`, byte, signed, `LDRD`/`STRD`,
//! `LDM`/`STM` with one address computation and per-register accesses in guest order; write-back
//! rules (skipped on a load with `rn == rd`), post-index; `bus.set_pc(u.pc)` before every access
//! (I5). Semantics are those of `ldst_exec`, `arm_ldst_half` and `arm_ldm`; the tests compare
//! registers and the ordered access log on a logging bus. The arm does not look at
//! `bus.stale()`: the executor loop does, after a store (I6).
//!
//! Registered statically in `exec::run_sb` (family `k::Family::Mem`). The signature is frozen.

use super::bus::SbBus;
use super::uop::Uop;

/// Execute one load/store uop on the registers `r` through `bus`.
pub fn exec_mem<B: SbBus>(r: &mut [u32; 16], bus: &mut B, u: &Uop) {
    let _ = (r, bus);
    unreachable!(
        "uop kind {} has no executor arm yet (OWNER W1-XMEM)",
        u.kind
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::uop::k;
    use crate::machine::SysBus;

    #[test]
    #[should_panic(expected = "W1-XMEM")]
    fn the_skeleton_has_no_arm_and_says_who_owns_it() {
        exec_mem(
            &mut [0; 16],
            &mut SysBus::new(),
            &Uop::new(k::LDR_IMM, 0x1000),
        );
    }
}
