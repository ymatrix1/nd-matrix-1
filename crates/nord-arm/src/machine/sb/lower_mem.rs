//! OWNER: W1-MEM. Memory instruction lowering: ARM word to `Uop` (skeleton: everything `GENERIC`).
//!
//! Spec (design note 3.1, 4.2, WP W1-MEM): word/byte loads and stores with immediate and
//! register offsets (`rs` flags b0 U, b1 P, b2 WB = `!P || W`), halfword/signed/double (`rs` b3 =
//! register offset), `LDM`/`STM` with S = 0 and a non-empty list (popcount in `sh`, list in
//! `imm`), pc-relative literal loads folded to `MOV_IMM` through `cx.fold` (it registers the
//! literal's page and word in the trace snapshot; `None` falls back to `LDR_ABS` or `Generic`).
//! Forms the interpreter can fault on stay [`Lowered::Generic`]: odd `Rd` for `ldrd`/`strd`,
//! `r14` pairs, an empty register list, the S bit, `rd == 15` loads (those are block enders and
//! never reach here). The trace builder (not this function) allocates the early exit of every
//! store uop and stores its index in `Uop::cond` (see `uop.rs`).
//!
//! Rule for `r15` (I2): `rn == 15` is folded (literal load through `cx.fold`, or `LDR_ABS`) or
//! `Generic`; `rd == 15` for a store, `rm == 15`, `r15` in an `STM` list (`LDM` with `r15` ends the
//! block), `rd >= 14` of `ldrd`/`strd` and any pc base with write-back are `Generic`.
//! `decode::lower_or_generic` overrules a uop that still names `r15` (`decode::touches_r15`), but
//! that is a backstop, not the plan. An instruction that lowers to several uops puts `n = 1` on
//! the first and `n = 0` on the others.
//!
//! The function is registered statically in `decode::lower_insn`; land the body here only.

use super::decode::{LowerCtx, Lowered};

/// Lower the load/store instruction of `cx.insn`: push unconditional uops to `cx.out` and return
/// [`Lowered::Done`], or push nothing and return [`Lowered::Generic`].
pub fn lower_mem(cx: &mut LowerCtx<'_>) -> Lowered {
    let _ = cx; // skeleton: the work package fills this in
    Lowered::Generic
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::decode::decode;
    use crate::machine::sb::trace::BuildOpts;

    #[test]
    fn the_skeleton_pushes_nothing_and_falls_back_to_generic() {
        let opts = BuildOpts::default();
        let mut out = Vec::new();
        let mut fold = |_: u32| None;
        let mut cx = LowerCtx {
            insn: decode(0x1000, 0xE591_0000), // ldr r0,[r1]
            out: &mut out,
            fold: &mut fold,
            opts: &opts,
        };
        assert_eq!(lower_mem(&mut cx), Lowered::Generic);
        assert!(out.is_empty());
    }
}
