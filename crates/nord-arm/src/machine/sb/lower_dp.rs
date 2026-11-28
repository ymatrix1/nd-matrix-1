//! OWNER: W1-DP. Data-processing lowering: ARM word to one `Uop` (or `GENERIC`).
//!
//! Spec: design note 3.1 and 11.3, work package W1-DP. [`lower_dp`] lowers every data-processing
//! word the predecoder classifies as `op::DP_BASE + k::dp(form, opcode, s)` (96 ids), choosing
//! the specialised id of `k::MOV_IMM..=k::ORR_IMM` where one exists and the generic id
//! `k::dp(form, opcode, s)` otherwise. It never pushes more than one uop and never pushes a uop
//! and then returns [`Lowered::Generic`].
//!
//! # Fields of the uop
//!
//! | Field | Rule |
//! |---|---|
//! | `rd` | destination; 0 for `tst/teq/cmp/cmn` (the Rd field is SBZ there and the interpreter ignores it) |
//! | `rn` | first operand; 0 for `mov/mvn` (the interpreter reads it and drops it) |
//! | `rm`, `rs` | form 1: `rm`; form 2: `rm` and `rs`; form 0 (immediate): both 0 |
//! | `imm` | form 0: `arm_imm(imm8, rot)`, the rotated immediate; else 0 |
//! | `sh` form 0 | bit 0 = 1 when `rot == 0` (the shifter carry is then the old C) |
//! | `sh` form 1 | `type << 5 \| amount` with the raw 5-bit amount: LSR/ASR #0 (= 32) and ROR #0 (= RRX) are resolved by `alu::shift(.., imm = true)`; 0 for a plain register |
//! | `sh` form 2 | `type << 5` |
//! | `sh` specialised | 0, except `TST_IMM`, which keeps the form 0 rule (`TST_GUARD_I` carries it) |
//! | `n`, `cond`, `pc` | 1, 0 (the registry wraps the predicate), the instruction address |
//!
//! Specialised ids, all with a plain operand (an immediate, or `Rm` with `lsl #0`):
//! `mov` (`MOV_IMM`, `MOV_REG`), `mvn` (`MVN_IMM`), `add` (`ADD_IMM`, `ADD_REG`), `sub`
//! (`SUB_IMM`, `SUB_REG`), `orr` (`ORR_IMM`) when not S; `cmp` (`CMP_IMM`, `CMP_REG`) and `tst`
//! (`TST_IMM`, `TST_REG`), which are always S. An S form of `mov/add/sub` stays generic.
//!
//! # `r15` (I2)
//!
//! An ARM-state instruction reads `r15` as its own address + 8, except that a register-shifted
//! operand (form 2) reads address + 12. No uop but `GENERIC*`/`EXIT*` names `r15`, so:
//!
//! * `add rd,pc,#x` (not S) folds to `MOV_IMM rd,#(pc + 8 + x)` and `mov rd,pc` (not S, `lsl
//!   #0`) to `MOV_IMM rd,#(pc + 8)`: nothing a register or flag holds influences the result.
//!   The fold does not depend on `BuildOpts::fold_literals` (it reads no memory).
//! * Every other pc operand (`rn` of an op that reads it, `rm`, `rs`; any S form; every form 2
//!   instruction) returns [`Lowered::Generic`]. Form 2 has no fold at all: the shifted value is
//!   not a constant, and the +12 bias differs from the +8 of the folds.
//! * `rd == 15` of an op that writes `rd` is a block ender and also returns `Generic` here
//!   (`decode::lower_family` sends it to `lower_flow`; `mov pc,rm` becomes `EXIT_REG_ARM` there).
//!   Test ops with an `Rd` field of 15 are not enders and lower normally with `rd = 0`.
//!
//! `decode::lower_or_generic` also overrules any uop that still names `r15`
//! (`decode::touches_r15`): a backstop, not the plan.
//!
//! # Mutant sites
//!
//! Test builds carry the mutant sites `0..=10` of the block `0..=99` reserved for this package
//! in `mutants.rs` (W3-VERIF registers the names): [`pick`] returns the wrong value when the
//! mutant is active. The table is [`MUTANT_SITES`]; `classes::every_mutant_site_changes_a_decision`
//! proves each one is reachable.
//!
//! The function is registered statically in `decode::lower_insn`; this file holds the body only.

use super::decode::{Insn, LowerCtx, Lowered};
use super::uop::{k, Uop};
use crate::cpu::alu::arm_imm;
use crate::cpu::predecode::op;

#[cfg(test)]
mod classes;
#[cfg(test)]
mod golden;
#[cfg(test)]
mod support;

/// The data-processing opcode field (bits 24..21) of the instructions this file names.
///
/// VERIFIED: `AND 0, EOR 1, SUB 2, RSB 3, ADD 4, ADC 5, SBC 6, RSC 7, TST 8, TEQ 9, CMP 10,
/// CMN 11, ORR 12, MOV 13, BIC 14, MVN 15` = the match arms of `Cpu::dp_exec`
/// (`cpu/arm_dp.rs`) and the ids of `predecode::op::dp`, checked by
/// `tests::the_opcode_constants_are_the_ones_the_predecoder_dispatches_on` (max err 0).
mod opc {
    pub const SUB: u32 = 2;
    pub const ADD: u32 = 4;
    pub const TST: u32 = 8;
    pub const CMP: u32 = 10;
    pub const CMN: u32 = 11;
    pub const ORR: u32 = 12;
    pub const MOV: u32 = 13;
    pub const MVN: u32 = 15;
}

/// `r15` as a register number.
const PC: u8 = 15;

/// How far ahead of its own address an ARM-state instruction reads `r15`.
///
/// VERIFIED: `Cpu::step` and `Cpu::exec_decoded` set `r[15] = pc + 8` before the handler runs
/// (`cpu/mod.rs`, `cpu/predecode.rs`), checked by
/// `classes::folded_constants_equal_what_the_interpreter_writes` over 600 random words and the
/// wrap-around addresses, max err 0. A register-shifted operand reads 12:
/// `classes::a_register_shifted_pc_reads_twelve_ahead_so_form_2_has_no_fold`.
const PC_AHEAD: u32 = 8;

/// Mutant sites of this file: `(id, what the wrong version does)`. Ids are in the block
/// `0..=99` reserved for W1-DP in `mutants.rs`.
#[cfg(test)]
pub(crate) const MUTANT_SITES: &[(u32, &str)] = &[
    (0, "dp-imm-rot0-flag-dropped"),
    (1, "dp-imm-rot0-flag-always-set"),
    (2, "dp-imm-rotate-dropped"),
    (3, "dp-pc-fold-reads-pc-plus-4"),
    (4, "dp-reg-shift-type-dropped"),
    (5, "dp-rsr-rm-rs-swapped"),
    (6, "dp-sub-imm-emitted-as-add-imm"),
    (7, "dp-cmp-reg-emitted-as-tst-reg"),
    (8, "dp-s-bit-ignored-by-specialised-ids"),
    (9, "dp-pc-fold-accepts-shifted-mov"),
    (10, "dp-pc-fold-accepts-adds"),
];

/// `wrong` when mutant `id` is active on this thread (test builds), else `right`.
#[cfg(test)]
fn pick<T>(id: u32, wrong: T, right: T) -> T {
    if super::mutants::is_active(id) {
        wrong
    } else {
        right
    }
}

/// `right`: mutant sites exist in test builds only.
#[cfg(not(test))]
#[inline(always)]
fn pick<T>(_id: u32, _wrong: T, right: T) -> T {
    right
}

/// Lower the data-processing instruction of `cx.insn`: push unconditional uops to `cx.out` and
/// return [`Lowered::Done`], or push nothing and return [`Lowered::Generic`].
pub fn lower_dp(cx: &mut LowerCtx<'_>) -> Lowered {
    match plan(&cx.insn) {
        Some(uop) => {
            cx.out.push(uop);
            Lowered::Done
        }
        None => Lowered::Generic,
    }
}

/// The one uop of `insn`, or `None` when the instruction stays `GENERIC`.
fn plan(insn: &Insn) -> Option<Uop> {
    // The predecoder's class is the source of truth: it is what the interpreter dispatches on.
    let id = insn
        .entry
        .op
        .checked_sub(op::DP_BASE)
        .filter(|id| *id < k::MOV_IMM)?;
    let (form, opcode, s) = (u32::from(id >> 5), u32::from(id >> 1 & 0xF), id & 1 != 0);
    let test = (opc::TST..=opc::CMN).contains(&opcode);
    let reads_rn = !matches!(opcode, opc::MOV | opc::MVN);
    let (rd, rn, rm, rs) = (insn.rd(), insn.rn(), insn.rm(), insn.rs());
    if rd == PC && !test {
        return None; // a block ender: `lower_flow`
    }
    if let Some(value) = pc_constant(insn, form, opcode, s) {
        let mut u = Uop::new(k::MOV_IMM, insn.pc);
        (u.rd, u.imm) = (rd, value);
        return Some(u);
    }
    if (reads_rn && rn == PC) || (form != 0 && rm == PC) || (form == 2 && rs == PC) {
        return None; // I2: only the two folds above may read the pc
    }
    let w = insn.word;
    let (mut sh, imm) = match form {
        0 => (rot0_flag(w), rotated_imm(w)),
        1 => (shift_by_imm(w), 0),
        _ => (((w >> 5 & 3) << 5) as u8, 0),
    };
    let spec = specialised(form, opcode, s, form == 0 || sh == 0);
    let kind = spec.unwrap_or(k::dp(form, opcode, s));
    if spec.is_some() && kind != k::TST_IMM {
        sh = 0; // a specialised uop has no shifter field, bar the carry bit `TST_GUARD_I` carries
    }
    let mut u = Uop::new(kind, insn.pc);
    u.rd = if test { 0 } else { rd };
    u.rn = if reads_rn { rn } else { 0 };
    (u.rm, u.rs) = match form {
        0 => (0, 0),
        1 => (rm, 0),
        _ => pick(5, (rs, rm), (rm, rs)),
    };
    (u.sh, u.imm) = (sh, imm);
    Some(u)
}

/// The rotated immediate operand of a form 0 word.
fn rotated_imm(w: u32) -> u32 {
    pick(2, w & 0xFF, arm_imm(w & 0xFF, w >> 8 & 0xF))
}

/// `sh` bit 0 of a form 0 uop: 1 when `rot == 0`, where the shifter carry is the old C.
fn rot0_flag(w: u32) -> u8 {
    let rot0 = u8::from(w >> 8 & 0xF == 0);
    pick(0, 0, pick(1, 1, rot0))
}

/// `sh` of a form 1 uop: `type << 5 | amount`, the amount still raw (0 means 32 or RRX).
fn shift_by_imm(w: u32) -> u8 {
    let (ty, amount) = (w >> 5 & 3, w >> 7 & 0x1F);
    pick(4, amount, ty << 5 | amount) as u8
}

/// The value `add rd,pc,#x` or `mov rd,pc` writes, which no register or flag influences.
fn pc_constant(insn: &Insn, form: u32, opcode: u32, s: bool) -> Option<u32> {
    let ahead = insn.pc.wrapping_add(pick(3, 4, PC_AHEAD));
    match (form, opcode) {
        (0, opc::ADD) if insn.rn() == PC && (!s || pick(10, true, false)) => {
            Some(ahead.wrapping_add(rotated_imm(insn.word)))
        }
        (1, opc::MOV) if insn.rm() == PC && !s && pick(9, true, insn.word >> 4 & 0xFF == 0) => {
            Some(ahead)
        }
        _ => None,
    }
}

/// The specialised id (`k::MOV_IMM..=k::ORR_IMM`) that replaces `k::dp(form, opcode, s)`, if
/// there is one. `plain` is true for an immediate and for `Rm` with `lsl #0`.
fn specialised(form: u32, opcode: u32, s: bool, plain: bool) -> Option<u8> {
    let non_s = !s || pick(8, true, false);
    let imm = form == 0;
    if !plain || form == 2 {
        return None;
    }
    Some(match (opcode, imm) {
        (opc::MOV, true) if non_s => k::MOV_IMM,
        (opc::MOV, false) if non_s => k::MOV_REG,
        (opc::MVN, true) if non_s => k::MVN_IMM,
        (opc::ADD, true) if non_s => k::ADD_IMM,
        (opc::ADD, false) if non_s => k::ADD_REG,
        (opc::SUB, true) if non_s => pick(6, k::ADD_IMM, k::SUB_IMM),
        (opc::SUB, false) if non_s => k::SUB_REG,
        (opc::CMP, true) => k::CMP_IMM,
        (opc::CMP, false) => pick(7, k::TST_REG, k::CMP_REG),
        (opc::TST, true) => k::TST_IMM,
        (opc::TST, false) => k::TST_REG,
        (opc::ORR, true) if non_s => k::ORR_IMM,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::support::{dp, imm_word, reg_word, rsr_word, AL, PC0};
    use super::*;
    use crate::machine::sb::decode::{decode, lower_or_generic};
    use crate::machine::sb::trace::BuildOpts;

    #[test]
    fn the_opcode_constants_are_the_ones_the_predecoder_dispatches_on() {
        for (opcode, name) in [
            (opc::SUB, "sub"),
            (opc::ADD, "add"),
            (opc::TST, "tst"),
            (opc::CMP, "cmp"),
            (opc::CMN, "cmn"),
            (opc::ORR, "orr"),
            (opc::MOV, "mov"),
            (opc::MVN, "mvn"),
        ] {
            let e = decode(PC0, imm_word(dp(AL, opcode, true, 1, 2), 0, 3)).entry;
            assert_eq!(e.op, op::dp(0, opcode, true), "{name}");
        }
        // the literal words of the assembler agree with the encoder
        let word = |opcode, s, rn, rd| dp(AL, opcode, s, rn, rd);
        assert_eq!(imm_word(word(opc::MOV, false, 0, 3), 0, 0x40), 0xE3A0_3040);
        assert_eq!(imm_word(word(opc::ADD, false, 15, 0), 0, 4), 0xE28F_0004);
        assert_eq!(reg_word(word(opc::MOV, false, 0, 1), 0, 0, 2), 0xE1A0_1002);
        assert_eq!(rsr_word(word(opc::ADD, false, 1, 0), 3, 0, 2), 0xE081_0312);
    }

    #[test]
    fn the_registry_wraps_a_conditional_lowering_in_one_pred_and_falls_back_to_generic() {
        let opts = BuildOpts::default();
        let run = |word: u32| {
            let mut out = Vec::new();
            let mut fold = |_: u32| None;
            let mut cx = LowerCtx {
                insn: decode(PC0, word),
                out: &mut out,
                fold: &mut fold,
                opts: &opts,
            };
            let lowered = lower_or_generic(&mut cx);
            (lowered, out)
        };
        let (lowered, out) = run(0x1281_0001); // addne r0,r1,#1
        assert_eq!(lowered, Lowered::Done);
        assert_eq!(
            (out.len(), out[0].kind, out[0].cond, out[0].imm),
            (2, k::PRED, 1, 1)
        );
        assert_eq!((out[1].kind, out[1].n), (k::ADD_IMM, 1));
        let (lowered, out) = run(0xE1A0_F001); // mov pc,r1: the registry sends it to lower_flow
        assert_eq!(
            (lowered, out),
            (Lowered::Generic, vec![Uop::generic(0xE1A0_F001, PC0, 0)])
        );
        let (_, out) = run(0xE28F_0004); // folded: no uop names r15
        assert_eq!((out.len(), out[0].kind), (1, k::MOV_IMM));
    }
}
