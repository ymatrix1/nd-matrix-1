//! OWNER: W1-MEM. pc-relative literal loads (`ldr rd,[pc,#off]` and its byte and halfword
//! cousins): folded to `MOV_IMM` when the literal's word can be read at build time, a runtime
//! `LDR_ABS` when it cannot (word loads on an aligned address only), `GENERIC` otherwise.
//!
//! Why this is exact: the interpreter reads `pc + 8` as the base (`Cpu::exec_decoded` sets
//! `r[15] = pc + 8`), so the address of a `P = 1`, `W = 0`, immediate-offset load from `pc` is a
//! build-time constant. The folded word is registered in the trace snapshot by `fold` (I1: the
//! trace depends on code words and literal words only), so a store to the literal's page
//! revalidates or drops the trace. Anything that writes `pc` back (post-index, `W = 1`), a
//! register offset, a store and `LDRD` are not literals and stay `GENERIC` (I2).

use super::super::decode::{Insn, LowerCtx};
use super::super::uop::{k, Uop};
use super::{flags, m, FLAG_P, FLAG_U, FLAG_WB};
use crate::cpu::predecode::op;

/// Width and extension of the loaded value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Word,
    Byte,
    Half,
    SignedHalf,
    SignedByte,
}

/// The constant address and shape of a literal load, or `None` when `insn` is not one.
fn literal_of(insn: &Insn) -> Option<(Shape, u32)> {
    let w = insn.word;
    let fl = flags(w);
    if insn.rn() != 15 || insn.rd() == 15 || fl & FLAG_P == 0 || fl & FLAG_WB != 0 {
        return None;
    }
    let (shape, off) = match insn.entry.op {
        op::LDR if w >> 25 & 1 == 0 => (Shape::Word, w & 0xFFF),
        op::LDRB if w >> 25 & 1 == 0 => (Shape::Byte, w & 0xFFF),
        op::HALF if w >> 20 & 1 != 0 && w >> 22 & 1 != 0 => {
            let shape = match w >> 5 & 3 {
                1 => Shape::Half,
                2 => Shape::SignedByte,
                _ => Shape::SignedHalf,
            };
            (shape, (w >> 4 & 0xF0) | (w & 0xF))
        }
        _ => return None,
    };
    // VERIFIED: the base of a pc-relative access is pc + 8 = cpu/predecode.rs `exec_decoded`
    // (`r[15] = pc.wrapping_add(8)`), checked by semantics::literals_fold_to_the_interpreter_value_and_otherwise_load_at_run_time
    // (600 literals at 6 pcs including the address-space wrap, 0 mismatches against `Cpu::step`).
    let base = insn.pc.wrapping_add(if m(118) { 4 } else { 8 });
    let addr = if (fl & FLAG_U != 0) != m(121) {
        base.wrapping_add(off)
    } else {
        base.wrapping_sub(off)
    };
    Some((shape, addr))
}

/// What the interpreter loads from `addr` given the aligned word `word` at `addr & !3`
/// (`Cpu::load32` rotates, `read16` aligns down, `read8` takes the byte, little endian).
fn value(shape: Shape, word: u32, addr: u32) -> u32 {
    let byte = word >> (8 * (addr & 3)) & 0xFF;
    let half = word >> (8 * (addr & if m(122) { 3 } else { 2 })) & 0xFFFF;
    match shape {
        Shape::Word if m(119) => word,
        Shape::Word => word.rotate_right(8 * (addr & 3)),
        Shape::Byte => byte,
        Shape::Half => half,
        Shape::SignedHalf => half as u16 as i16 as i32 as u32,
        Shape::SignedByte if m(120) => byte,
        Shape::SignedByte => byte as u8 as i8 as i32 as u32,
    }
}

/// Lower a literal load, or `None` when `cx.insn` is not one or nothing exact can be emitted.
/// `cx.fold` is called only for an instruction that will become a `MOV_IMM`.
pub(super) fn lower(cx: &mut LowerCtx<'_>) -> Option<Uop> {
    let (shape, addr) = literal_of(&cx.insn)?;
    let (pc, rd) = (cx.insn.pc, cx.insn.rd());
    if cx.opts.fold_literals {
        if let Some(word) = (cx.fold)(addr & !3) {
            let mut u = Uop::new(k::MOV_IMM, pc);
            u.rd = rd;
            u.imm = value(shape, word, addr);
            return Some(u);
        }
    }
    // Not folded: a word load from an aligned constant address is `LDR_ABS`; the executor then
    // reads it through the bus at run time, exactly as the interpreter does.
    (shape == Shape::Word && addr & 3 == 0).then(|| {
        let mut u = Uop::new(k::LDR_ABS, pc);
        u.rd = rd;
        u.imm = if m(123) { addr.wrapping_add(4) } else { addr };
        u
    })
}

#[cfg(test)]
mod tests {
    use super::super::semantics::{check_on, unmutated};
    use super::*;
    use crate::machine::sb::decode::{decode, lower_or_generic, Lowered};
    use crate::machine::sb::testkit::{random_cpu, Rng};
    use crate::machine::sb::trace::BuildOpts;

    #[test]
    fn the_loaded_value_follows_the_interpreter_rotate_align_and_extend_rules() {
        let w = 0x8877_66F5; // bytes F5 66 77 88 (little endian)
        assert_eq!(value(Shape::Word, w, 0x100), 0x8877_66F5);
        assert_eq!(
            value(Shape::Word, w, 0x101),
            0xF588_7766,
            "rotate right by 8"
        );
        assert_eq!(value(Shape::Word, w, 0x103), 0x7766_F588);
        assert_eq!(value(Shape::Byte, w, 0x100), 0xF5);
        assert_eq!(value(Shape::Byte, w, 0x103), 0x88);
        assert_eq!(value(Shape::Half, w, 0x100), 0x66F5);
        assert_eq!(
            value(Shape::Half, w, 0x101),
            0x66F5,
            "odd address aligns down"
        );
        assert_eq!(value(Shape::Half, w, 0x102), 0x8877);
        assert_eq!(value(Shape::SignedHalf, w, 0x100), 0x66F5);
        assert_eq!(value(Shape::SignedHalf, 0x0000_80F5, 0x100), 0xFFFF_80F5);
        assert_eq!(value(Shape::SignedByte, w, 0x100), 0xFFFF_FFF5);
        assert_eq!(value(Shape::SignedByte, w, 0x102), 0x77);
    }

    #[test]
    fn only_pre_indexed_immediate_loads_from_pc_without_write_back_are_literals() {
        let lit = |w: u32| literal_of(&decode(0x2000, w));
        assert_eq!(lit(0xE59F_0004), Some((Shape::Word, 0x200C))); // ldr r0,[pc,#4]
        assert_eq!(lit(0xE51F_0004), Some((Shape::Word, 0x2004))); // ldr r0,[pc,#-4]
        assert_eq!(lit(0xE5DF_0001), Some((Shape::Byte, 0x2009))); // ldrb r0,[pc,#1]
        assert_eq!(lit(0xE1DF_00B2), Some((Shape::Half, 0x200A))); // ldrh r0,[pc,#2]
        assert_eq!(lit(0xE1DF_00D1), Some((Shape::SignedByte, 0x2009))); // ldrsb r0,[pc,#1]
        assert_eq!(lit(0xE1DF_00F2), Some((Shape::SignedHalf, 0x200A))); // ldrsh r0,[pc,#2]
        for no in [
            0xE5BF_0004, // ldr r0,[pc,#4]!  (write-back)
            0xE49F_0004, // ldr r0,[pc],#4   (post-index)
            0xE79F_0001, // ldr r0,[pc,r1]   (register offset)
            0xE58F_0004, // str r0,[pc,#4]
            0xE59F_F004, // ldr pc,[pc,#4]   (ender)
            0xE59E_0004, // ldr r0,[lr,#4]   (not pc based)
            0xE1CF_00F2, // strd r0,[pc,#2]
            0xE19F_00B1, // ldrh r0,[pc,r1]  (register offset)
        ] {
            assert_eq!(lit(no), None, "{no:#010x}");
        }
    }

    /// A literal that writes `pc` back is not one: `lower` declines before it calls `fold`, and
    /// the registry emits `GENERIC`. The same word without the write-back folds and equals
    /// `Cpu::step`, so the decline is the write-back's doing and not a refusal of every literal.
    #[test]
    fn a_literal_load_that_writes_the_pc_back_stays_generic_where_the_plain_one_folds() {
        let mut rng = Rng::new(0x79);
        let opts = BuildOpts {
            fold_literals: true,
            ..BuildOpts::default()
        };
        for (plain, pre_wb, post) in [
            (0xE59F_0004, 0xE5BF_0004, 0xE49F_0004), // ldr r0,[pc,#4] | [pc,#4]! | [pc],#4
            (0xE55F_0004, 0xE57F_0004, 0xE45F_0004), // ldrb r0,[pc,#-4] | ! | post-indexed
            (0xE1DF_00B2, 0xE1FF_00B2, 0xE0DF_00B2), // ldrh r0,[pc,#2] | ! | post-indexed
        ] {
            for pc in [0x1000, 0xC000_0FFC, 0xFFFF_FFF8] {
                let (seed, cpu) = (rng.next(), random_cpu(&mut rng, pc));
                let seen = check_on(plain, cpu.clone(), seed, true, unmutated)
                    .unwrap_or_else(|e| panic!("{e}"));
                assert!(seen.is_some_and(|s| s.folded), "{plain:#010x} at {pc:#x}");
                for word in [pre_wb, post] {
                    let mut out = Vec::new();
                    let mut fold =
                        |a: u32| -> Option<u32> { panic!("fold({a:#x}) for {word:#010x}") };
                    let lowered = lower_or_generic(&mut LowerCtx {
                        insn: decode(pc, word),
                        out: &mut out,
                        fold: &mut fold,
                        opts: &opts,
                    });
                    assert_eq!(lowered, Lowered::Generic, "{word:#010x} at {pc:#x}");
                    assert_eq!(out, vec![Uop::generic(word, pc, 0)]);
                    let seen = check_on(word, cpu.clone(), seed, true, unmutated)
                        .unwrap_or_else(|e| panic!("{e}"));
                    assert!(seen.is_none(), "{word:#010x} must not lower");
                }
            }
        }
    }
}
