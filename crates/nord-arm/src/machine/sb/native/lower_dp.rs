//! OWNER: W2-NDP. Native lowering of data processing (skeleton: every uop is `Unsupported`, so the
//! native trace ends with a `Generic` exit and the interpreter runs it).
//!
//! Spec (design note 9, WP W2-NDP): all 96 generic ids, the specialised ids, shifts, flag arithmetic from SSA operands (a compare followed by a guard fuses to one `icmp`), conditions and the multiplies that have kind ids. Flag defs overwritten later in the trace are dead (the rule of `flagopt.rs`). Each arm must equal the T2 executor on random
//! states (`native::difftest`); an unsupported shape returns `Err(Unsupported)`, never a
//! guess (I8).

use super::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::Uop;

/// Emit the native code of `u` (data processing).
pub fn lower_dp(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
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
            for kind in [k::ADD_IMM, k::MOV_REG, k::dp(0, 4, true)] {
                assert_eq!(
                    lower_dp(cx, &Uop::new(kind, 0x1000)),
                    Err(Unsupported),
                    "kind {kind}"
                );
            }
            cx.exit_generic(0x1000);
        });
    }
}
