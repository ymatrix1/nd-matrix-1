//! OWNER: W2-MUL. Multiply lowering: `MUL`, `MLA` and their S forms to one `Uop` (or `GENERIC`).
//!
//! Spec (design note 3.1, work package W2-MUL): [`lower_mul`] lowers the words the predecoder
//! classifies as `op::MUL` whose opcode field (bits 23..21) is 0 (`MUL`) or 1 (`MLA`) to the id
//! `k::mul(acc, s)` (`108..=111`, family `Dp`, executed by `exec_alu::exec_mul_with`). It pushes
//! at most one uop and never pushes a uop and then returns [`Lowered::Generic`].
//!
//! # Fields of the uop
//!
//! | Field | Rule |
//! |---|---|
//! | `kind` | `k::mul(acc, s)` = `MUL + acc * 2 + s` |
//! | `rd` | destination, instruction bits 19..16 (the interpreter's `hi`) |
//! | `rn` | accumulator, bits 15..12 (the interpreter's `lo`); 0 for `MUL`, where the field is SBZ and ignored |
//! | `rs`, `rm` | bits 11..8 and bits 3..0 |
//! | `n`, `cond`, `pc` | 1, 0 (the registry wraps the predicate), the instruction address |
//!
//! Semantics, as `Cpu::arm_mul` (cases `0 | 1`): `rd = rm * rs (+ rn)` modulo 2^32, every source
//! read before `rd` is written (so `rd == rm`, `rd == rs`, `rn == rd` are exact); with S, N and Z
//! come from the result and C and V are unchanged (ARMv5 `MULS`). The interpreter charges one
//! instruction and nothing else: `Exit::done` counts it like any other instruction.
//!
//! # What stays `GENERIC`
//!
//! * the long multiplies `UMULL`, `UMLAL`, `SMULL`, `SMLAL` (opcode field 4..7): two result
//!   registers and flags from a 64-bit value. They are not lowered at the proof level of this
//!   file (no executor arm), see `docs/specs/arm0-superblocks.md`;
//! * the ARMv5E halfword multiplies (`SMLAxy`, `SMULxy`, ...), which the predecoder does not
//!   class as `op::MUL`;
//! * any word with `r15` in `Rd`, `Rs`, `Rm` or the `bits 15..12` field (`pc` reads `pc + 8`, I2;
//!   those words are also block enders for the predecoder, `ends_block`).
//!
//! # Mutant sites
//!
//! Test builds carry the sites `900..=907` of the block `900..=949` reserved for this package
//! (`mutants.rs`): [`pick`] returns the wrong value when the mutant is active. The table is
//! [`MUTANT_SITES`]; `diff::every_mutant_site_is_killed` proves each one is observed.
//!
//! Registered statically in `decode::lower_insn` (`LowerFamily::Mul`); this file holds the body.

use super::decode::{Insn, LowerCtx, Lowered};
use super::uop::{k, Uop};
use crate::cpu::predecode::op;

#[cfg(test)]
mod diff;

/// `r15` as a register number.
const PC: u8 = 15;

/// Mutant sites of this file: `(id, what the wrong version does)`.
#[cfg(test)]
pub(crate) const MUTANT_SITES: &[(u32, &str)] = &[
    (900, "mul-rd-and-rn-fields-swapped"),
    (901, "mul-acc-bit-ignored"),
    (902, "mul-s-bit-ignored"),
    (903, "mul-long-multiply-accepted"),
    (904, "mul-pc-operand-accepted"),
    (905, "mul-s-always-set"),
    (906, "mul-kind-acc-and-s-swapped"),
    (907, "mla-accumulator-register-dropped"),
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

/// Lower the multiply of `cx.insn`: push one unconditional uop to `cx.out` and return
/// [`Lowered::Done`], or push nothing and return [`Lowered::Generic`].
pub fn lower_mul(cx: &mut LowerCtx<'_>) -> Lowered {
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
    if insn.entry.op != op::MUL {
        return None;
    }
    let w = insn.word;
    // opcode field (bit 24 is clear for `op::MUL`): 0 MUL, 1 MLA, 4..=7 the long multiplies, 2
    // and 3 undefined encodings the interpreter runs as long multiplies
    let opcode = w >> 21 & 0xF;
    if opcode > 1 && !pick(903, true, false) {
        return None;
    }
    let s = w >> 20 & 1 != 0;
    let (hi, lo) = (insn.rn(), insn.rd()); // bits 19..16 and 15..12
    let (rs, rm) = (insn.rs(), insn.rm());
    let pc_operand = [hi, lo, rs, rm].contains(&PC);
    if pc_operand && !pick(904, true, false) {
        return None; // I2: only `GENERIC` reads the pc
    }
    let acc = pick(901, false, opcode & 1 != 0);
    let s = pick(902, false, pick(905, true, s));
    let kind = pick(906, k::mul(s, acc), k::mul(acc, s));
    let mut u = Uop::new(kind, insn.pc);
    u.rd = pick(900, lo, hi);
    u.rn = if acc && !pick(907, true, false) {
        pick(900, hi, lo)
    } else {
        0
    };
    (u.rs, u.rm) = (rs, rm);
    Some(u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::decode::{decode, lower_or_generic, LowerFamily};
    use crate::machine::sb::trace::BuildOpts;

    const PC0: u32 = 0x2000;

    /// `mul`/`mla` word, condition always: `rd` bits 19..16, `rn` 15..12, `rs` 11..8, `rm` 3..0.
    fn word(acc: bool, s: bool, rd: u32, rn: u32, rs: u32, rm: u32) -> u32 {
        0xE000_0090 | u32::from(acc) << 21 | u32::from(s) << 20 | rd << 16 | rn << 12 | rs << 8 | rm
    }

    fn lowered(word: u32) -> (Lowered, Vec<Uop>) {
        let opts = BuildOpts::default();
        let (mut out, mut fold) = (Vec::new(), |_: u32| None);
        let mut cx = LowerCtx {
            insn: decode(PC0, word),
            out: &mut out,
            fold: &mut fold,
            opts: &opts,
        };
        let l = lower_mul(&mut cx);
        (l, out)
    }

    fn uop(kind: u8, rd: u8, rn: u8, rs: u8, rm: u8) -> Uop {
        Uop {
            rd,
            rn,
            rs,
            rm,
            ..Uop::new(kind, PC0)
        }
    }

    #[test]
    fn the_four_forms_lower_to_their_ids_with_the_field_table() {
        // mul r1,r2,r0 / muls r1,r2,r0 (Rd = bits 19..16; bits 15..12 = SBZ, ignored)
        assert_eq!(
            lowered(0xE001_0092),
            (Lowered::Done, vec![uop(k::MUL, 1, 0, 0, 2)])
        );
        assert_eq!(
            lowered(0xE011_0092),
            (Lowered::Done, vec![uop(k::MULS, 1, 0, 0, 2)])
        );
        // mla r1,r2,r0,r3 / mlas
        assert_eq!(
            lowered(0xE021_3092),
            (Lowered::Done, vec![uop(k::MLA, 1, 3, 0, 2)])
        );
        assert_eq!(
            lowered(0xE031_3092),
            (Lowered::Done, vec![uop(k::MLAS, 1, 3, 0, 2)])
        );
        // all four fields distinct, nothing copied from the wrong field
        assert_eq!(
            lowered(word(true, true, 4, 5, 6, 7)).1,
            vec![uop(k::MLAS, 4, 5, 6, 7)]
        );
        // MUL with a non-zero SBZ field: the field is dropped
        assert_eq!(
            lowered(word(false, false, 4, 5, 6, 7)).1,
            vec![uop(k::MUL, 4, 0, 6, 7)]
        );
    }

    #[test]
    fn long_and_halfword_multiplies_and_pc_operands_stay_generic() {
        for (w, why) in [
            (0xE080_1392, "umull"),
            (0xE0A0_1392, "umlal"),
            (0xE0C2_1093, "smull"),
            (0xE0E2_1093, "smlal"),
            (0xE001_F092, "mul with 15..12 = pc"),
            (word(false, false, 15, 0, 1, 2), "mul pc,.."),
            (word(true, false, 1, 3, 15, 2), "mla rs = pc"),
            (word(false, true, 1, 0, 2, 15), "muls rm = pc"),
            (word(true, false, 1, 15, 2, 3), "mla rn = pc"),
            (0xE100_0080, "smlabb (not op::MUL)"),
            (0xE280_0001, "add"),
        ] {
            assert_eq!(lowered(w), (Lowered::Generic, vec![]), "{why}");
        }
    }

    #[test]
    fn the_registry_routes_multiplies_here_and_wraps_a_conditional_one_in_a_pred() {
        assert_eq!(decode(PC0, 0xE001_0092).lower_family(), LowerFamily::Mul);
        assert_eq!(decode(PC0, 0xE0C2_1093).lower_family(), LowerFamily::Mul);
        let opts = BuildOpts::default();
        let run = |w: u32| {
            let (mut out, mut fold) = (Vec::new(), |_: u32| None);
            let mut cx = LowerCtx {
                insn: decode(PC0, w),
                out: &mut out,
                fold: &mut fold,
                opts: &opts,
            };
            (lower_or_generic(&mut cx), out)
        };
        let (l, out) = run(0x0011_0092); // mulseq r1,r2,r0
        assert_eq!(l, Lowered::Done);
        assert_eq!(
            (out.len(), out[0].kind, out[0].cond, out[0].imm),
            (2, k::PRED, 0, 1)
        );
        assert_eq!((out[1].kind, out[1].n, out[1].cond), (k::MULS, 1, 0));
        let (l, out) = run(0xE001_F092); // pc in the SBZ field: an ender, `GENERIC`
        assert_eq!(
            (l, out),
            (Lowered::Generic, vec![Uop::generic(0xE001_F092, PC0, 0)])
        );
        let (l, out) = run(0xE0C2_1093); // smull
        assert_eq!(
            (l, out),
            (Lowered::Generic, vec![Uop::generic(0xE0C2_1093, PC0, 0)])
        );
    }
}
