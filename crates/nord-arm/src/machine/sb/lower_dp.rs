//! OWNER: W1-DP. Data-processing lowering: ARM word to `Uop` (skeleton: everything `GENERIC`).
//!
//! Spec (design note 3.1, 11.3, WP W1-DP): lower all 96 generic ids (`k::dp(form, opcode, s)`),
//! the specialised ids `MOV_IMM..=ORR_IMM`, the form-0 `rot == 0` flag in `sh` bit 0 (shifter
//! carry = C), form-1 `sh = type << 5 | amount` with the special encodings resolved by
//! `alu::shift(.., imm = true)`, the form-2 `pc + 12` rule for `rn == 15`/`rm == 15`; fold `add
//! rd,pc,#x` and `mov rd,pc` to `MOV_IMM` (I2: no uop but `GENERIC*`/`EXIT*` reads `r15`);
//! `rd == 15` and S-with-`rd == 15` return [`Lowered::Generic`] (the registry sends them to
//! `lower_flow` first). Logic ops keep V and use the shifter carry; arithmetic uses
//! `alu::add_c` semantics. Never push a uop and then return `Generic`.
//!
//! Rule for `r15` (I2): every use of `r15` that is not folded to a constant is `Generic`: `rn` or
//! `rm` equal to 15 in any form but the two folds above, `rs == 15`, and `S` with a pc operand.
//! `decode::lower_or_generic` overrules a uop that still names `r15` (`decode::touches_r15`),
//! but that is a backstop, not the plan. Specialised non-S uops keep `sh == 0` (the fuser relies
//! on it); an instruction that lowers to several uops puts `n = 1` on the first and `n = 0` on
//! the others (a conditional instruction still retires once: `done` is static).
//!
//! The function is registered statically in `decode::lower_insn`; land the body here only.

use super::decode::{LowerCtx, Lowered};

/// Lower the data-processing instruction of `cx.insn`: push unconditional uops to `cx.out` and
/// return [`Lowered::Done`], or push nothing and return [`Lowered::Generic`].
pub fn lower_dp(cx: &mut LowerCtx<'_>) -> Lowered {
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
            insn: decode(0x1000, 0xE280_0001), // add r0,r0,#1
            out: &mut out,
            fold: &mut fold,
            opts: &opts,
        };
        assert_eq!(lower_dp(&mut cx), Lowered::Generic);
        assert!(out.is_empty());
    }
}
