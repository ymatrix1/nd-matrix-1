//! OWNER: W1-MEM (tests). Word generators of the `lower_mem` tests: each builds an ARM word from
//! chosen fields and the uop it must lower to, so an expectation never comes from decoding the
//! word. Shared by `decisions.rs` and `semantics.rs`.

use super::{FLAG_P, FLAG_REG, FLAG_U, FLAG_WB};
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::uop::{k, Uop};

/// Address of the instruction in every generated case.
pub(super) const PC: u32 = 0x1000;

fn cond(rng: &mut Rng) -> u32 {
    (if rng.chance(3) { rng.below(14) } else { 14 }) << 28
}

fn offset(rng: &mut Rng, bits: u32) -> u32 {
    let max = (1 << bits) - 1;
    if rng.chance(6) {
        rng.pick(&[0, 1, 2, 3, max, max - 1])
    } else {
        rng.below(max + 1)
    }
}

/// The wb rule the interpreter uses for single and half transfers, written out.
fn expected_rs(p: bool, u: bool, w: bool) -> u8 {
    let wb = !p || w;
    (if u { FLAG_U } else { 0 }) | (if p { FLAG_P } else { 0 }) | (if wb { FLAG_WB } else { 0 })
}

/// A word/byte transfer (`reg`: shifted register offset) and the uop it must lower to.
pub(super) fn single(rng: &mut Rng, load: bool, byte: bool, reg: bool) -> (u32, Uop) {
    const KINDS: [[[u8; 2]; 2]; 2] = [
        [[k::LDR_IMM, k::LDRB_IMM], [k::STR_IMM, k::STRB_IMM]],
        [[k::LDR_REG, k::LDRB_REG], [k::STR_REG, k::STRB_REG]],
    ];
    let (p, u, w) = (rng.chance(2), rng.chance(2), rng.chance(2));
    let (rn, rd, rm) = (rng.below(15), rng.below(15), rng.below(15));
    let (ty, amount, off) = (rng.below(4), rng.below(32), offset(rng, 12));
    let low = if reg { amount << 7 | ty << 5 | rm } else { off };
    let word = cond(rng)
        | 1 << 26
        | (reg as u32) << 25
        | (p as u32) << 24
        | (u as u32) << 23
        | (byte as u32) << 22
        | (w as u32) << 21
        | (load as u32) << 20
        | rn << 16
        | rd << 12
        | low;
    let mut want = Uop::new(KINDS[reg as usize][!load as usize][byte as usize], PC);
    (want.rd, want.rn, want.rs) = (rd as u8, rn as u8, expected_rs(p, u, w));
    if reg {
        (want.rm, want.sh) = (rm as u8, (ty << 5 | amount) as u8);
    } else {
        want.imm = off;
    }
    (word, want)
}

/// A halfword, signed or double transfer; `sh` is the `S H` field (1 `H`, 2 `S`, 3 `SH`) and
/// `load` selects `ldrh/ldrsb/ldrsh` against `strh/ldrd/strd`.
pub(super) fn half(rng: &mut Rng, load: bool, sh: u32, reg: bool) -> (u32, Uop) {
    let kind = [
        [0, k::STRH, k::LDRD, k::STRD],
        [0, k::LDRH, k::LDRSB, k::LDRSH],
    ][load as usize][sh as usize];
    let pair = matches!(kind, k::LDRD | k::STRD);
    let (p, u, w) = (rng.chance(2), rng.chance(2), rng.chance(2));
    let rd = if pair {
        2 * rng.below(7)
    } else {
        rng.below(15)
    };
    let (rn, rm, off) = (rng.below(15), rng.below(15), offset(rng, 8));
    let low = if reg {
        rng.below(16) << 8 | rm // bits 11..8 are ignored by the interpreter
    } else {
        (off & 0xF0) << 4 | (off & 0xF)
    };
    let word = cond(rng)
        | (p as u32) << 24
        | (u as u32) << 23
        | (!reg as u32) << 22
        | (w as u32) << 21
        | (load as u32) << 20
        | rn << 16
        | rd << 12
        | 1 << 7
        | sh << 5
        | 1 << 4
        | low;
    let mut want = Uop::new(kind, PC);
    (want.rd, want.rn, want.rs) = (rd as u8, rn as u8, expected_rs(p, u, w));
    if reg {
        (want.rm, want.rs) = (rm as u8, want.rs | FLAG_REG);
    } else {
        want.imm = off;
    }
    (word, want)
}

/// `ldm`/`stm` with a non-empty list of r0..r14 and S = 0.
pub(super) fn block(rng: &mut Rng, load: bool) -> (u32, Uop) {
    let list = match rng.below(4) {
        0 => 1 << rng.below(15),
        1 => 0x7FFF,
        _ => rng.below(0x7FFF) + 1,
    };
    let (p, u, w, rn) = (rng.chance(2), rng.chance(2), rng.chance(2), rng.below(15));
    let word = cond(rng)
        | 1 << 27
        | (p as u32) << 24
        | (u as u32) << 23
        | (w as u32) << 21
        | (load as u32) << 20
        | rn << 16
        | list;
    let mut want = Uop::new(if load { k::LDM } else { k::STM }, PC);
    want.rn = rn as u8;
    want.rs =
        (if u { FLAG_U } else { 0 }) | (if p { FLAG_P } else { 0 }) | (if w { FLAG_WB } else { 0 });
    want.sh = (0..16).filter(|b| list >> b & 1 != 0).count() as u8;
    want.imm = list;
    (word, want)
}

/// A pc-relative immediate load `ldr/ldrb/ldrh/ldrsb/ldrsh rd,[pc,#+-off]`. Returns the word and
/// the address it reads (a constant of the pc and the offset).
pub(super) fn literal(rng: &mut Rng, pc: u32) -> (u32, u32) {
    let (up, rd) = (rng.chance(2), rng.below(15));
    let (word, off) = match rng.below(5) {
        0..=2 => {
            let (off, byte) = (offset(rng, 12), rng.chance(3));
            (1 << 26 | 1 << 24 | (byte as u32) << 22 | 1 << 20 | off, off)
        }
        _ => {
            let (off, sh) = (offset(rng, 8), 1 + rng.below(3));
            (
                1 << 24
                    | 1 << 22
                    | 1 << 20
                    | (off & 0xF0) << 4
                    | 1 << 7
                    | sh << 5
                    | 1 << 4
                    | (off & 0xF),
                off,
            )
        }
    };
    let word = cond(rng) | word | (up as u32) << 23 | 15 << 16 | rd << 12;
    let base = pc.wrapping_add(8);
    (
        word,
        if up {
            base.wrapping_add(off)
        } else {
            base.wrapping_sub(off)
        },
    )
}

/// Any word of the load/store encoding space with every register field free (r15 included).
pub(super) fn wild(rng: &mut Rng) -> u32 {
    let w = rng.next() & 0x0FFF_FFFF | cond(rng);
    match rng.below(4) {
        0 => w & !(7 << 25) | 2 << 25,
        1 => w & !(7 << 25) & !0x10 | 3 << 25,
        2 => w & !(7 << 25) | 4 << 25,
        _ => (w & !(7 << 25) | 0x90) | if w >> 5 & 3 == 0 { 1 << 5 } else { 0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::predecode::op;
    use crate::machine::sb::decode::decode;

    #[test]
    fn each_generator_yields_words_the_decoder_puts_in_its_class() {
        let mut rng = Rng::new(0x99);
        let ops = |w: u32| decode(PC, w).entry.op;
        for _ in 0..200 {
            let (load, byte, reg) = (rng.chance(2), rng.chance(2), rng.chance(2));
            let want = [[op::STR, op::STRB], [op::LDR, op::LDRB]][load as usize][byte as usize];
            assert_eq!(ops(single(&mut rng, load, byte, reg).0), want);
            let sh = 1 + rng.below(3);
            assert_eq!(ops(half(&mut rng, load, sh, reg).0), op::HALF);
            assert_eq!(ops(block(&mut rng, load).0), op::LDM);
            let (word, _) = literal(&mut rng, PC);
            assert!(matches!(ops(word), op::LDR | op::LDRB | op::HALF));
            assert_eq!(word >> 16 & 0xF, 15, "a literal is based on pc");
            let w = wild(&mut rng);
            assert!(matches!(
                ops(w),
                op::LDR | op::LDRB | op::STR | op::STRB | op::LDM | op::HALF
            ));
        }
    }

    #[test]
    fn the_expected_uop_follows_the_chosen_fields() {
        let mut rng = Rng::new(0x9A);
        let (word, want) = single(&mut rng, true, true, false);
        assert_eq!(
            (want.kind, want.pc, want.n, want.cond),
            (k::LDRB_IMM, PC, 1, 0)
        );
        assert_eq!(want.imm, word & 0xFFF);
        let (word, want) = half(&mut rng, false, 2, true);
        assert_eq!(
            (want.kind, want.rd % 2, want.rs & FLAG_REG),
            (k::LDRD, 0, FLAG_REG)
        );
        assert_eq!(want.rm as u32, word & 0xF);
        let (word, want) = block(&mut rng, false);
        assert_eq!((want.kind, want.imm), (k::STM, word & 0xFFFF));
        assert_eq!(want.sh as u32, (word & 0xFFFF).count_ones());
    }
}
