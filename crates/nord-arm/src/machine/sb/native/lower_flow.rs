//! OWNER: W2-NMEM. Native lowering of control uops (skeleton: every uop is `Unsupported`, so the
//! native trace ends with a `Generic` exit and the interpreter runs it).
//!
//! Spec (design note 9, WP W2-NMEM): `PRED` (skip a block when the condition fails), `GUARD` (a side exit), `EXIT` (static targets through `NCtx::exit`), `LOOP_BACK` (a native back-edge guarded by `cpu.insns + n_guest <= ctl.limit`), the fused uops (each equal to its unfused expansion) and `GENERIC` (always `Unsupported`: it is the point where native code hands over). Each arm must equal the T2 executor on random
//! states (`native::difftest`); an unsupported shape returns `Err(Unsupported)`, never a
//! guess (I8).

use super::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::Uop;

/// Emit the native code of `u` (control uops).
pub fn lower_flow(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
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
            for kind in [k::PRED, k::GUARD, k::EXIT, k::LOOP_BACK, k::CMP_GUARD_I] {
                assert_eq!(
                    lower_flow(cx, &Uop::new(kind, 0x1000)),
                    Err(Unsupported),
                    "kind {kind}"
                );
            }
            cx.exit_generic(0x1000);
        });
    }
}
