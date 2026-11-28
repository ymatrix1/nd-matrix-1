//! OWNER: W2-NMEM. Native lowering of loads and stores (skeleton: every uop is `Unsupported`, so the
//! native trace ends with a `Generic` exit and the interpreter runs it).
//!
//! Spec (design note 9, WP W2-NMEM): the page-table fast path with the masks of `abi.rs` (`e = table[a >> 12]`, mapped and no hook flag, host address `arena - 4096 + (index << 12) + (a & 0xFFF)`); an access wider than the page remainder, or any failed check, is `side_exit_generic_if` at the instruction pc with the state exact before it. A store that can leave `stale_code` non-empty needs no extra code (the page was marked `HOOK_C`, so the store takes the slow path). Each arm must equal the T2 executor on random
//! states (`native::difftest`); an unsupported shape returns `Err(Unsupported)`, never a
//! guess (I8).

use super::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::Uop;

/// Emit the native code of `u` (loads and stores).
pub fn lower_mem(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    let _ = (cx, u);
    Err(Unsupported)
}

#[cfg(test)]
mod tests {
    use super::super::emit::testing::build_and_verify;
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::uop::k;

    #[test]
    fn the_skeleton_declines_every_uop_of_its_family() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        build_and_verify(&sb, |cx| {
            for kind in [k::LDR_IMM, k::STR_IMM, k::LDM, k::STM] {
                assert_eq!(
                    lower_mem(cx, &Uop::new(kind, 0x1000)),
                    Err(Unsupported),
                    "kind {kind}"
                );
            }
            cx.exit_generic(0x1000);
        });
    }
}
