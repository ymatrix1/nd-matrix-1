//! Test support of `lower_dp`: word encoders, a random word of a given predecode class, and a
//! one-call lowering helper. `cfg(test)` only; shared by `lower_dp::tests` and `classes`.

use super::{lower_dp, Lowered};
use crate::machine::sb::decode::{decode, LowerCtx};
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::trace::BuildOpts;
use crate::machine::sb::uop::Uop;

/// The condition field "always".
pub(super) const AL: u32 = 14;

/// The fields every data-processing word has: `<cond> <opcode>{s} rd, rn, <operand 2>`.
pub(super) fn dp(cond: u32, opcode: u32, s: bool, rn: u32, rd: u32) -> u32 {
    cond << 28 | opcode << 21 | u32::from(s) << 20 | rn << 16 | rd << 12
}

/// Operand 2 of form 0: `#rotated(imm8, rot)`.
pub(super) fn imm_word(head: u32, rot: u32, imm8: u32) -> u32 {
    head | 1 << 25 | rot << 8 | imm8
}

/// Operand 2 of form 1: `rm, <ty> #amount` (`ty` 0 lsl, 1 lsr, 2 asr, 3 ror).
pub(super) fn reg_word(head: u32, amount: u32, ty: u32, rm: u32) -> u32 {
    head | amount << 7 | ty << 5 | rm
}

/// Operand 2 of form 2: `rm, <ty> rs`.
pub(super) fn rsr_word(head: u32, rs: u32, ty: u32, rm: u32) -> u32 {
    head | rs << 8 | ty << 5 | 1 << 4 | rm
}

/// Run `lower_dp` on `word` at `pc` with a context that has no literal folding and an empty
/// `out`; returns what it decided and what it pushed.
pub(super) fn lower(word: u32, pc: u32) -> (Lowered, Vec<Uop>) {
    let opts = BuildOpts::default();
    let (mut out, mut fold) = (Vec::new(), |_: u32| None);
    let mut cx = LowerCtx {
        insn: decode(pc, word),
        out: &mut out,
        fold: &mut fold,
        opts: &opts,
    };
    let lowered = lower_dp(&mut cx);
    (lowered, out)
}

/// The address the golden tests lower at.
pub(super) const PC0: u32 = 0x2000;

/// A `Uop` of `kind` at [`PC0`] with the given `(rd, rn, rm, rs, sh, imm)`, `n = 1`.
pub(super) fn uop(kind: u8, f: (u8, u8, u8, u8, u8, u32)) -> Uop {
    let mut u = Uop::new(kind, PC0);
    (u.rd, u.rn, u.rm, u.rs, u.sh, u.imm) = f;
    u
}

/// The one uop `word` lowers to at [`PC0`] (fails when it is `Generic` or pushes more).
pub(super) fn done(word: u32) -> Uop {
    let (lowered, out) = lower(word, PC0);
    assert_eq!((lowered, out.len()), (Lowered::Done, 1), "{word:#010x}");
    out[0]
}

/// Asserts that `word` stays `GENERIC` at [`PC0`] and pushes nothing.
pub(super) fn generic(word: u32) {
    let (lowered, out) = lower(word, PC0);
    assert_eq!((lowered, out.len()), (Lowered::Generic, 0), "{word:#010x}");
}

/// A register number, biased towards the pc so every class meets pc operands often.
fn reg(rng: &mut Rng) -> u32 {
    if rng.chance(5) {
        15
    } else {
        rng.below(16)
    }
}

/// A random word of the generic id `id` (`k::dp(form, opcode, s)`), condition 0..=14, with the
/// register fields biased to 15 and the shift fields biased to the special amount 0. The
/// predecoder only gives S-less test opcodes to the "miscellaneous" space, so those ids (12 of
/// the 96) have no word that decodes to them: the word is built all the same and the caller
/// checks the class.
pub(super) fn class_word(rng: &mut Rng, id: u8) -> u32 {
    let (form, opcode, s) = (u32::from(id >> 5), u32::from(id >> 1 & 0xF), id & 1 != 0);
    let (cond, rn, rd, rm) = (rng.below(15), reg(rng), reg(rng), reg(rng));
    let ty = if rng.chance(3) { 0 } else { rng.below(4) };
    match form {
        0 => {
            let rot = if rng.chance(4) { 0 } else { rng.below(16) };
            imm_word(dp(cond, opcode, s, rn, rd), rot, rng.below(256))
        }
        1 => {
            let amount = if rng.chance(3) { 0 } else { rng.below(32) };
            reg_word(dp(cond, opcode, s, rn, rd), amount, ty, rm)
        }
        _ => rsr_word(dp(cond, opcode, s, rn, rd), reg(rng), ty, rm),
    }
}

/// An instruction address: aligned, mostly low, sometimes within 16 bytes of the 32-bit wrap.
pub(super) fn some_pc(rng: &mut Rng) -> u32 {
    if rng.chance(8) {
        0u32.wrapping_sub(4 * (1 + rng.below(4)))
    } else {
        0x1000 + 4 * rng.below(0x1_0000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::predecode::op;

    #[test]
    fn encoders_build_the_words_an_assembler_would() {
        let mov_r3 = dp(AL, 13, false, 0, 3);
        assert_eq!(imm_word(mov_r3, 0, 0x40), 0xE3A0_3040); // mov r3,#0x40
        assert_eq!(reg_word(dp(AL, 4, true, 1, 0), 3, 0, 2), 0xE091_0182); // adds r0,r1,r2,lsl #3
        assert_eq!(rsr_word(dp(AL, 13, false, 0, 0), 2, 3, 1), 0xE1A0_0271); // mov r0,r1,ror r2
        assert_eq!(imm_word(dp(0, 4, false, 0, 1), 0, 4), 0x0280_1004); // addeq r1,r0,#4
    }

    #[test]
    fn class_words_decode_to_their_class_and_the_s_less_tests_are_the_unreachable_ids() {
        let mut rng = Rng::new(3);
        let (mut reachable, mut misc) = (0, 0);
        for id in 0..96u8 {
            let unreachable = (8..=11).contains(&(id >> 1 & 0xF)) && id & 1 == 0;
            for _ in 0..20 {
                let w = class_word(&mut rng, id);
                let decoded = decode(0x1000, w).entry.op;
                assert!(w >> 28 < 15, "{w:#010x}");
                if unreachable {
                    assert!(decoded < op::DP_BASE, "{w:#010x}: op {decoded}");
                    misc += 1;
                } else {
                    assert_eq!(decoded, op::DP_BASE + id, "{w:#010x}");
                    reachable += 1;
                }
            }
        }
        assert_eq!((reachable, misc), (84 * 20, 12 * 20));
    }

    #[test]
    fn addresses_are_aligned_and_reach_the_wrap() {
        let mut rng = Rng::new(9);
        let pcs: Vec<u32> = (0..400).map(|_| some_pc(&mut rng)).collect();
        assert!(pcs.iter().all(|pc| pc & 3 == 0));
        assert!(pcs.iter().any(|pc| *pc > 0xFFFF_FFF0));
        assert!(pcs.iter().any(|pc| *pc < 0x2_0000));
    }
}
