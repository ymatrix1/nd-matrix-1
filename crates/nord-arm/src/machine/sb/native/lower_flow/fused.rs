//! OWNER: W2-NMEM. Native fused uops (`CMP_GUARD_I`, `CMP_GUARD_R`, `TST_GUARD_I`, `MOV_PAIR`,
//! `LDR_PAIR`): each is its unfused expansion (`uop::fused::expand`), lowered through the
//! context's own lowerer, so the compare and the moves are whatever `lower_dp` does and the
//! guard and the loads are this package's.
//!
//! `Unsupported` is answered before anything is emitted: both halves are lowered into a scratch
//! function first (`lowerable`), because the first half may succeed where the second does not. The
//! pair retires two instructions: while the second half is lowered `retired` already counts the
//! first, so a side exit of the second half (`LDR_PAIR`'s second load on a slow page) leaves with
//! the state exact before instruction two; the walk adds `Uop::n` itself afterwards, so `retired`
//! is put back to the value it had on entry.

use super::super::lower_mem::mutant;
use crate::machine::sb::native::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::fused::expand;
use crate::machine::sb::uop::Uop;
use cranelift_codegen::ir::{Function, UserFuncName};
use cranelift_frontend::FunctionBuilderContext;

/// Whether every uop of `parts` lowers with the lowerer of `cx` (nothing is emitted in `cx`).
fn lowerable(cx: &NCtx<'_>, parts: &[Uop]) -> bool {
    let mut func =
        Function::with_name_signature(UserFuncName::default(), cx.b.func.signature.clone());
    let mut fctx = FunctionBuilderContext::new();
    let mut scratch = NCtx::new(&mut func, &mut fctx, cx.sb());
    scratch.set_lowerer(cx.lowerer());
    let lower = scratch.lowerer();
    parts.iter().enumerate().all(|(i, p)| {
        scratch.retired = i as u16;
        lower(&mut scratch, p).is_ok()
    })
}

/// Lower a fused uop as its expansion.
pub(super) fn lower_fused(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    let Some(parts) = expand(u) else {
        return Err(Unsupported);
    };
    if !lowerable(cx, &parts) {
        return Err(Unsupported);
    }
    let lower = cx.lowerer();
    let start = cx.retired;
    for (i, part) in parts.iter().enumerate() {
        cx.retired = start + if mutant(887) { 0 } else { i as u16 };
        if lower(cx, part).is_err() {
            // cannot happen: `lowerable` ran the same lowerer on the same uops
            cx.exit_generic(part.pc);
            return Ok(());
        }
    }
    cx.retired = start;
    Ok(())
}

#[cfg(test)]
mod unit {
    use super::*;
    use crate::machine::sb::native::emit::testing::build_and_verify;
    use crate::machine::sb::native::lower_mem::tests::one;
    use crate::machine::sb::uop::k;

    #[test]
    fn a_uop_that_is_not_fused_is_declined_and_retired_is_untouched() {
        let sb = one(Uop::new(k::LDR_IMM, 0x1000));
        build_and_verify(&sb, |cx| {
            let before = cx.b.func.dfg.num_insts();
            assert_eq!(
                lower_fused(cx, &Uop::new(k::ADD_IMM, 0x1000)),
                Err(Unsupported)
            );
            assert_eq!((cx.retired, cx.b.func.dfg.num_insts()), (0, before));
            cx.exit_generic(0x1000);
        });
    }
}
