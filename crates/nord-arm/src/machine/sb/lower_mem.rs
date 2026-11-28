//! OWNER: W1-MEM. Memory instruction lowering: ARM word to `Uop` (design note 3.1, 4.2, WP W1-MEM).
//!
//! One load or store instruction lowers to exactly one uop (`n = 1`), or to nothing
//! ([`Lowered::Generic`]; the registry then emits `GENERIC`). Field layout, as the table of the
//! design note says (the `rs` bits are U, P, WB, REG = bits 0 to 3):
//!
//! | Instruction | kind | `rd` | `rn` | `rm` | `rs` | `sh` | `imm` |
//! |---|---|---|---|---|---|---|---|
//! | `ldr/ldrb/str/strb rd,[rn,#imm12]` | `LDR_IMM`, `LDRB_IMM`, `STR_IMM`, `STRB_IMM` | Rd | Rn | 0 | U, P, WB | 0 | offset (12 bit) |
//! | the same with `[rn,rm,shift #n]` | `LDR_REG`, `LDRB_REG`, `STR_REG`, `STRB_REG` | Rd | Rn | Rm | U, P, WB | `type << 5 \| amount` (raw, 0..31) | 0 |
//! | `ldrh/ldrsh/ldrsb/strh/ldrd/strd` | `LDRH`, `LDRSH`, `LDRSB`, `STRH`, `LDRD`, `STRD` | Rd | Rn | Rm (REG) | U, P, WB, REG | 0 | offset (8 bit) |
//! | `ldm/stm` (any of the four modes) | `LDM`, `STM` | 0 | Rn | 0 | U, P, WB | popcount | register list |
//! | `ldr rd,[pc,#imm]` folded | `MOV_IMM` | Rd | 0 | 0 | 0 | 0 | the loaded value |
//! | `ldr rd,[pc,#imm]` unfolded | `LDR_ABS` | Rd | 0 | 0 | 0 | 0 | absolute address (word aligned) |
//!
//! * `WB` is "the base register is written back": `!P || W` for the single and half transfers
//!   (the interpreter's `wb`), plain `W` for `ldm/stm` (the interpreter's `w`). The executor
//!   still skips it for a load with `rn == rd` (and for `ldm` with `rn` in the list).
//! * The shift of a register offset stays raw: `alu::shift(type, amount, r[rm], C, imm = true)`
//!   resolves `LSR/ASR #0 = 32` and `ROR #0 = RRX` at execution time, like the interpreter.
//! * `uop.cond` is left 0: the trace builder stores the index of the early exit of a store there
//!   (I6); `wrap_pred` adds the `PRED` of a conditional instruction.
//! * A load into `pc` ends the block and never reaches this file (`lower_flow`); the same
//!   encodings are refused here anyway. So is a register offset with bit 4 set: the decoder
//!   classes it `SLOW` (undefined media space), which is a block ender too.
//!
//! Rule for `r15` (I2): a uop never names `r15`. `rn == 15` is a literal load (module `literal`:
//! folded through `cx.fold`, or `LDR_ABS`) or `Generic`; `rd == 15` of a store, `rm == 15`,
//! `r15` in an `STM` list and `rd >= 14` of `ldrd`/`strd` are `Generic`. Also `Generic` (the
//! interpreter's own handler runs them): an odd `Rd` of `ldrd`/`strd`, the S bit of `ldm/stm`
//! and an empty register list. `decode::lower_or_generic` overrules any uop that still names
//! `r15` (`decode::touches_r15`), but that is a backstop: [`lower_mem`] never needs it (test
//! `decisions::lower_mem_itself_never_emits_a_uop_that_names_r15`).
//!
//! The function is registered statically in `decode::lower_insn`; land the body here only.

use super::decode::{Insn, LowerCtx, Lowered};
use super::uop::{k, Uop};
use crate::cpu::predecode::op;

#[cfg(test)]
mod decisions;
mod literal;
#[cfg(test)]
mod model;
#[cfg(test)]
mod semantics;
#[cfg(test)]
mod words;

/// Bit 0 of `Uop::rs` of a memory uop: the offset is added (`U`).
///
/// VERIFIED: the layout of `rs` for memory uops = design note 3.1 field table (b0 U, b1 P, b2 WB,
/// b3 REG) with the write-back rule of `cpu/arm_mem.rs` (`ldst_exec`, `arm_ldst_half`, `arm_ldm`),
/// checked by `semantics::lowered_memory_uops_equal_the_interpreter_on_random_states` (1,800
/// random cases over every P/U/W combination, 0 mismatches against `Cpu::step`).
pub const FLAG_U: u8 = 1;
/// Bit 1 of `Uop::rs`: pre-indexed (`P`), the transfer address includes the offset.
pub const FLAG_P: u8 = 1 << 1;
/// Bit 2 of `Uop::rs`: the base register is written back (see the module docs).
pub const FLAG_WB: u8 = 1 << 2;
/// Bit 3 of `Uop::rs` of the half and double kinds: the offset is register `rm`, not `imm`.
pub const FLAG_REG: u8 = 1 << 3;

/// Mutation site of layer L5 (`mutants.rs`, block 100..=199 of this package): true only while
/// mutant `id` is switched on for this thread, and a constant `false` outside tests.
#[cfg(test)]
fn m(id: u32) -> bool {
    super::mutants::is_active(id)
}

#[cfg(not(test))]
const fn m(_id: u32) -> bool {
    false
}

/// `U`, `P` and `WB` of a single or half transfer word (`wb = !p || w`, as `ldst_exec` and
/// `arm_ldst_half` compute it).
fn flags(word: u32) -> u8 {
    let (u, p, w) = (
        word >> 23 & 1 != 0,
        word >> 24 & 1 != 0,
        word >> 21 & 1 != 0,
    );
    let wb = if m(102) { w } else { !p || w };
    let mut f = u as u8 | (p as u8) << 1 | (wb as u8) << 2;
    if m(100) {
        f ^= FLAG_U;
    }
    if m(101) {
        f ^= FLAG_P;
    }
    f
}

/// `ldr/ldrb/str/strb`, immediate or register offset (the base is not `pc`: see [`lower_mem`]).
fn word_byte(i: &Insn) -> Option<Uop> {
    let w = i.word;
    let (reg, load, byte) = (w >> 25 & 1 != 0, w >> 20 & 1 != 0, w >> 22 & 1 != 0);
    if i.rd() == 15 || (reg && i.rm() == 15) {
        return None;
    }
    let (load, byte) = (load ^ m(104), byte ^ m(103));
    let first = if reg { k::LDR_REG } else { k::LDR_IMM }; // LDR, LDRB, STR, STRB follow in order
    let mut u = Uop::new(first + 2 * !load as u8 + byte as u8, i.pc);
    (u.rd, u.rn, u.rs) = (i.rd(), i.rn(), flags(w));
    if reg {
        u.rm = i.rm();
        let ty = if m(105) { 0 } else { w >> 5 & 3 };
        let amount = (if m(106) { w >> 8 } else { w >> 7 }) & 0x1F;
        u.sh = (ty << 5 | amount) as u8;
    } else {
        u.imm = w & if m(107) { 0xFF } else { 0xFFF };
    }
    Some(u)
}

/// Halfword, signed and double transfers (the base is not `pc`).
fn half(i: &Insn) -> Option<Uop> {
    let w = i.word;
    let kind = match (w >> 20 & 1 != 0, w >> 5 & 3) {
        (true, 1) if m(109) => k::LDRSH,
        (true, 1) => k::LDRH,
        (true, 2) if m(110) => k::LDRSH,
        (true, 2) => k::LDRSB,
        (true, 3) if m(109) => k::LDRH,
        (true, 3) if m(110) => k::LDRSB,
        (true, 3) => k::LDRSH,
        (false, 1) => k::STRH,
        (false, 2) if m(111) => k::STRD,
        (false, 2) => k::LDRD,
        (false, 3) if m(111) => k::LDRD,
        (false, 3) => k::STRD,
        _ => return None, // not a halfword encoding (the decoder never sends these)
    };
    let pair = matches!(kind, k::LDRD | k::STRD);
    let reg = w >> 22 & 1 == 0;
    // r14 would pair with r15; an odd register is unpredictable and stays the interpreter's.
    if i.rd() == 15 || (reg && i.rm() == 15) || (pair && (i.rd() & 1 != 0 || i.rd() == 14)) {
        return None;
    }
    let mut u = Uop::new(kind, i.pc);
    (u.rd, u.rn, u.rs) = (i.rd(), i.rn(), flags(w));
    if reg {
        u.rm = i.rm();
        u.rs |= if m(112) { 0 } else { FLAG_REG };
    } else {
        u.imm = (if m(108) { 0 } else { w >> 4 & 0xF0 }) | (w & 0xF);
    }
    Some(u)
}

/// `ldm` and `stm` with S = 0 and a non-empty list without `r15` (the base is not `pc`).
fn block(i: &Insn) -> Option<Uop> {
    let w = i.word;
    let list = w & 0xFFFF;
    if w >> 22 & 1 != 0 || list == 0 || list & 0x8000 != 0 {
        return None;
    }
    let load = (w >> 20 & 1 != 0) ^ m(117);
    let mut u = Uop::new(if load { k::LDM } else { k::STM }, i.pc);
    u.rn = i.rn();
    let bit = |n: u32| (w >> n & 1) as u8;
    let (up, pre) = if m(115) {
        (bit(24), bit(23))
    } else {
        (bit(23), bit(24))
    };
    u.rs = up | pre << 1 | if m(116) { 0 } else { bit(21) << 2 }; // U, P, W
    let list = if m(113) { list & !1 } else { list };
    u.sh = (list.count_ones() as u8).wrapping_add(m(114) as u8);
    u.imm = list;
    Some(u)
}

/// Lower the load/store instruction of `cx.insn`: push unconditional uops to `cx.out` and return
/// [`Lowered::Done`], or push nothing and return [`Lowered::Generic`].
pub fn lower_mem(cx: &mut LowerCtx<'_>) -> Lowered {
    let i = cx.insn;
    let uop = if i.rn() == 15 {
        literal::lower(cx)
    } else {
        match i.entry.op {
            op::LDR | op::LDRB | op::STR | op::STRB => word_byte(&i),
            op::HALF => half(&i),
            op::LDM => block(&i),
            _ => None,
        }
    };
    match uop {
        Some(u) => {
            cx.out.push(u);
            Lowered::Done
        }
        None => Lowered::Generic,
    }
}

/// The mutants of this package (ids 100..=199 of `mutants.rs`): each is a wrong lowering that
/// `semantics::every_mutant_of_this_package_is_killed_by_its_semantic_suite` proves the
/// comparison against `Cpu::step` catches. W3-VERIF registers these pairs in `mutants::MUTANTS`.
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (100, "mem: U flag inverted"),
    (101, "mem: P flag inverted"),
    (102, "mem: write-back ignores the post-index rule"),
    (103, "word/byte: byte and word kinds swapped"),
    (104, "word/byte: load and store kinds swapped"),
    (105, "word/byte: register-offset shift type dropped"),
    (
        106,
        "word/byte: register-offset shift amount from bits 11..8",
    ),
    (107, "word/byte: immediate offset cut to 8 bits"),
    (108, "half: immediate offset high nibble dropped"),
    (109, "half: ldrh and ldrsh swapped"),
    (110, "half: ldrsb and ldrsh swapped"),
    (111, "half: ldrd and strd swapped"),
    (112, "half: register-offset flag not set"),
    (113, "block: r0 dropped from the register list"),
    (114, "block: popcount off by one"),
    (115, "block: P and U swapped"),
    (116, "block: write-back dropped"),
    (117, "block: ldm and stm swapped"),
    (118, "literal: base is pc + 4"),
    (119, "literal: word rotate dropped"),
    (120, "literal: signed byte not extended"),
    (121, "literal: offset sign inverted"),
    (122, "literal: halfword read at the unaligned byte"),
    (123, "literal: LDR_ABS address off by 4"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::decode::decode;
    use crate::machine::sb::trace::BuildOpts;

    fn lower(word: u32) -> (Lowered, Vec<Uop>) {
        let opts = BuildOpts::default();
        let mut out = Vec::new();
        let mut fold = |_: u32| None;
        let mut cx = LowerCtx {
            insn: decode(0x1000, word),
            out: &mut out,
            fold: &mut fold,
            opts: &opts,
        };
        (lower_mem(&mut cx), out)
    }

    #[test]
    fn a_plain_load_becomes_one_uop_and_a_non_memory_word_nothing() {
        let (r, out) = lower(0xE591_0004); // ldr r0,[r1,#4]
        assert_eq!(r, Lowered::Done);
        let mut want = Uop::new(k::LDR_IMM, 0x1000);
        (want.rn, want.rs, want.imm) = (1, FLAG_U | FLAG_P, 4);
        assert_eq!(out, vec![want]);
        for word in [0xE280_0001, 0xEAFF_FFFE, 0xE001_0092, 0xF57F_F05F] {
            assert_eq!(lower(word), (Lowered::Generic, vec![]), "{word:#010x}");
        }
    }

    #[test]
    fn the_flag_bits_are_the_documented_ones_and_write_back_follows_p_and_w() {
        assert_eq!((FLAG_U, FLAG_P, FLAG_WB, FLAG_REG), (1, 2, 4, 8));
        assert_eq!(flags(0xE591_0004), FLAG_U | FLAG_P); // [r1,#4]
        assert_eq!(flags(0xE5B1_0004), FLAG_U | FLAG_P | FLAG_WB); // [r1,#4]!
        assert_eq!(flags(0xE411_0004), FLAG_WB); // ldr r0,[r1],#-4
        assert_eq!(flags(0xE531_0004), FLAG_P | FLAG_WB); // [r1,#-4]!
    }
}
