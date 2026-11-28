//! OWNER: S0 (frozen). The instruction view the lowerers work on, the lowering context types
//! shared by `lower_dp` and `lower_mem`, and the static registry that routes an instruction to
//! its lowering family.
//!
//! Registration is static: [`Insn::lower_family`] decides, [`lower_insn`] calls the owner's
//! function (`lower_dp::lower_dp`, `lower_mem::lower_mem`), [`lower_or_generic`] adds the
//! predicate wrap and the `GENERIC` fallback. Control flow (every block ender: branches, `mov
//! pc,rm`, loads into `pc`, the `SLOW` classes) goes to `lower_flow` through `FlowCtx`, which the
//! trace builder drives. A work package that lands a family changes only its own file.

use super::lower_dp::lower_dp;
use super::lower_mem::lower_mem;
use super::trace::BuildOpts;
use super::uop::{k, Uop};
use crate::cpu::predecode::{decode_arm, ends_block, op, Entry, MASK_ALWAYS};
use crate::cpu::Cpu;

/// One ARM instruction at an address, with its predecode classification.
#[derive(Clone, Copy)]
pub struct Insn {
    pub pc: u32,
    pub word: u32,
    pub entry: Entry,
}

/// Classify `word` found at `pc`.
pub fn decode(pc: u32, word: u32) -> Insn {
    Insn {
        pc,
        word,
        entry: decode_arm(word),
    }
}

/// Which lowering function takes an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LowerFamily {
    /// A block ender: `lower_flow` (through the trace builder).
    Flow,
    /// Word, byte, half, double and block transfers that cannot end a block.
    Mem,
    /// Data processing that cannot end a block.
    Dp,
    /// No uop kind exists (multiplies): `GENERIC`.
    Generic,
}

impl Insn {
    /// Condition field (0..=15).
    pub fn cond(&self) -> u8 {
        (self.word >> 28) as u8
    }

    /// True when the instruction runs only under a condition (a `PRED` wraps its uops).
    pub fn is_cond(&self) -> bool {
        self.entry.mask != MASK_ALWAYS
    }

    pub fn rd(&self) -> u8 {
        (self.word >> 12 & 0xF) as u8
    }

    pub fn rn(&self) -> u8 {
        (self.word >> 16 & 0xF) as u8
    }

    pub fn rm(&self) -> u8 {
        (self.word & 0xF) as u8
    }

    pub fn rs(&self) -> u8 {
        (self.word >> 8 & 0xF) as u8
    }

    /// Operand form of a data-processing word: 0 immediate, 1 register shifted by an immediate,
    /// 2 register shifted by a register.
    pub fn form(&self) -> u32 {
        Cpu::dp_form(self.word)
    }

    /// True when the instruction may change `pc` other than to the next word, switch
    /// instruction set or fault (`predecode::ends_block`).
    pub fn is_ender(&self) -> bool {
        ends_block(&self.entry)
    }

    /// Target of a `B`/`BL` word (`pc + 8 + offset`, word aligned); `pc + 4` for anything else.
    pub fn branch_target(&self) -> u32 {
        if matches!(self.entry.op, op::B | op::BL) {
            let off = (((self.word & 0x00FF_FFFF) << 8) as i32 >> 6) as u32;
            self.pc.wrapping_add(8).wrapping_add(off) & !3
        } else {
            self.pc.wrapping_add(4)
        }
    }

    /// The static routing decision.
    pub fn lower_family(&self) -> LowerFamily {
        if self.is_ender() {
            return LowerFamily::Flow;
        }
        match self.entry.op {
            op::LDR | op::LDRB | op::STR | op::STRB | op::LDM | op::HALF => LowerFamily::Mem,
            id if id >= op::DP_BASE => LowerFamily::Dp,
            _ => LowerFamily::Generic,
        }
    }
}

/// What a lowering function did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lowered {
    /// Uops were pushed to `out`.
    Done,
    /// Nothing was pushed: the caller emits `GENERIC` (I8).
    Generic,
}

/// Context of `lower_dp` and `lower_mem`.
pub struct LowerCtx<'a> {
    pub insn: Insn,
    /// Unconditional uops are appended here; the predicate wrap is the caller's.
    pub out: &'a mut Vec<Uop>,
    /// Literal folding: the word at an address, registered in the trace's page list and word
    /// snapshot (`None` when the page is unmapped, hooked or over the page cap).
    pub fold: &'a mut dyn FnMut(u32) -> Option<u32>,
    pub opts: &'a BuildOpts,
}

/// Static registry for the non-ender families. Control flow is not handled here.
pub fn lower_insn(cx: &mut LowerCtx<'_>) -> Lowered {
    match cx.insn.lower_family() {
        LowerFamily::Mem => lower_mem(cx),
        LowerFamily::Dp => lower_dp(cx),
        LowerFamily::Flow | LowerFamily::Generic => Lowered::Generic,
    }
}

/// Wrap the uops pushed since `start` under a `PRED` when the instruction is conditional. A
/// conditional instruction whose condition fails still counts one instruction (static `done`).
pub fn wrap_pred(out: &mut Vec<Uop>, start: usize, insn: &Insn) {
    if insn.is_cond() && out.len() > start {
        let mut pred = Uop::new(k::PRED, insn.pc);
        pred.n = 0;
        pred.cond = insn.cond();
        pred.imm = (out.len() - start) as u32;
        out.insert(start, pred);
    }
}

/// I2 backstop. True when a data-processing or memory uop names `r15` as a register operand,
/// a destination, a base, or in a block-transfer list: such an instruction reads `pc + 8` (or
/// `pc + 12` for a register-shifted operand and for `STR`/`STM` of `pc`), which only `GENERIC`
/// (via `exec_decoded`) reproduces. `rs` is a register only for the data-processing family (in
/// the memory family it holds the U/P/WB/REG flags); `LDRD`/`STRD` also touch `rd + 1`.
pub fn touches_r15(u: &Uop) -> bool {
    match k::family(u.kind) {
        k::Family::Dp => u.rd == 15 || u.rn == 15 || u.rm == 15 || u.rs == 15,
        k::Family::Mem => {
            let pair = matches!(u.kind, k::LDRD | k::STRD) && u.rd >= 14;
            let list = matches!(u.kind, k::LDM | k::STM) && u.imm & 0x8000 != 0;
            u.rd == 15 || u.rn == 15 || u.rm == 15 || pair || list
        }
        _ => false,
    }
}

/// Close the lowering of one instruction whose uops start at `start` in `out`: predicate wrap
/// on success, one `GENERIC` uop (its early exit index is 0 until the trace builder allocates
/// the real one) when the lowerer declined or its uops break I2.
fn seal(out: &mut Vec<Uop>, start: usize, insn: &Insn, lowered: Lowered) -> Lowered {
    if lowered == Lowered::Done && !out[start..].iter().any(touches_r15) {
        wrap_pred(out, start, insn);
        Lowered::Done
    } else {
        out.truncate(start);
        out.push(Uop::generic(insn.word, insn.pc, 0));
        Lowered::Generic
    }
}

/// [`lower_insn`], then [`seal`]. Returns what was emitted. A lowerer that produces a uop
/// touching `r15` is overruled here, so I2 holds whatever a work package does.
pub fn lower_or_generic(cx: &mut LowerCtx<'_>) -> Lowered {
    let start = cx.out.len();
    let lowered = lower_insn(cx);
    seal(cx.out, start, &cx.insn, lowered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(pc: u32, word: u32) -> Insn {
        decode(pc, word)
    }

    #[test]
    fn fields_and_condition_of_a_data_processing_word() {
        let i = at(0x1000, 0xE081_2003); // add r2,r1,r3
        assert_eq!((i.cond(), i.rd(), i.rn(), i.rm()), (14, 2, 1, 3));
        assert_eq!((i.form(), i.is_cond(), i.is_ender()), (1, false, false));
        let i = at(0x1000, 0x0280_1004); // addeq r1,r0,#4
        assert_eq!((i.cond(), i.form(), i.is_cond()), (0, 0, true));
        let i = at(0x1000, 0xE1A0_0211); // mov r0,r1,lsl r2
        assert_eq!((i.form(), i.rs()), (2, 2));
        assert!(
            !at(0x1000, 0xF57F_F05F).is_cond(),
            "the unconditional space always runs"
        );
    }

    #[test]
    fn branch_targets_follow_the_offset_and_the_pc_plus_8_rule() {
        let base = 0xC000_0000;
        assert_eq!(at(base + 8, 0xEB00_0003).branch_target(), base + 28); // bl +3 words
        assert_eq!(at(base + 20, 0x1AFF_FFFB).branch_target(), base + 8); // bne back 5
        assert_eq!(at(base + 24, 0xEAFF_FFFE).branch_target(), base + 24); // b .
        assert_eq!(
            at(base, 0xE280_0001).branch_target(),
            base + 4,
            "not a branch"
        );
    }

    #[test]
    fn the_registry_routes_every_class_to_one_family() {
        let fam = |w: u32| at(0x1000, w).lower_family();
        assert_eq!(fam(0xE280_0001), LowerFamily::Dp); // add r0,r0,#1
        assert_eq!(fam(0xE150_0001), LowerFamily::Dp); // cmp r0,r1
        assert_eq!(fam(0xE591_0000), LowerFamily::Mem); // ldr r0,[r1]
        assert_eq!(fam(0xE5C1_0000), LowerFamily::Mem); // strb r0,[r1]
        assert_eq!(fam(0xE92D_4010), LowerFamily::Mem); // push {r4,lr}
        assert_eq!(fam(0xE0C0_00B0), LowerFamily::Mem); // strh
        assert_eq!(fam(0xEAFF_FFFE), LowerFamily::Flow); // b .
        assert_eq!(fam(0xE12F_FF1E), LowerFamily::Flow); // bx lr
        assert_eq!(fam(0xE1A0_F00E), LowerFamily::Flow); // mov pc,lr
        assert_eq!(fam(0xE591_F000), LowerFamily::Flow); // ldr pc,[r1]
        assert_eq!(fam(0xE10F_3000), LowerFamily::Flow); // mrs r3,cpsr (SLOW class)
        assert_eq!(fam(0xE001_0092), LowerFamily::Generic); // mul r1,r2,r0: no kind id
    }

    #[test]
    fn an_unlowered_instruction_becomes_one_generic_uop() {
        let opts = BuildOpts::default();
        let mut out = vec![Uop::new(k::MOV_IMM, 0x0FFC)];
        let mut fold = |_: u32| None;
        let mut cx = LowerCtx {
            insn: at(0x1000, 0xE001_0092), // mul r1,r2,r0: no kind id exists, so no lowerer takes it
            out: &mut out,
            fold: &mut fold,
            opts: &opts,
        };
        assert_eq!(lower_or_generic(&mut cx), Lowered::Generic);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1], Uop::generic(0xE001_0092, 0x1000, 0));
    }

    #[test]
    fn a_conditional_instruction_is_wrapped_in_a_pred_over_its_uops() {
        let insn = at(0x1000, 0x1280_0001); // addne r0,r0,#1
        let mut out = vec![Uop::new(k::MOV_IMM, 0x0FFC)];
        out.push(Uop::new(k::ADD_IMM, 0x1000));
        out.push(Uop::new(k::ADD_IMM, 0x1000));
        wrap_pred(&mut out, 1, &insn);
        assert_eq!(out.len(), 4);
        assert_eq!(
            (out[1].kind, out[1].cond, out[1].imm, out[1].n),
            (k::PRED, 1, 2, 0)
        );
        // unconditional instructions and empty lowerings are left alone
        let always = at(0x1000, 0xE280_0001);
        wrap_pred(&mut out, 1, &always);
        wrap_pred(&mut out, 4, &insn);
        assert_eq!(out.len(), 4);
    }
    #[test]
    fn a_uop_that_names_r15_is_overruled_into_one_generic() {
        let insn = at(0x1000, 0xE280_0001);
        for bad in [
            {
                let mut u = Uop::new(k::ADD_IMM, 0x1000);
                u.rn = 15;
                u
            },
            {
                let mut u = Uop::new(k::ADD_REG, 0x1000);
                u.rm = 15;
                u
            },
            {
                let mut u = Uop::new(k::dp(2, 4, false), 0x1000);
                u.rs = 15;
                u
            },
            {
                let mut u = Uop::new(k::STR_IMM, 0x1000);
                u.rd = 15;
                u
            },
            {
                let mut u = Uop::new(k::STM, 0x1000);
                u.imm = 0x8010;
                u
            },
            {
                let mut u = Uop::new(k::STRD, 0x1000);
                u.rd = 14;
                u
            },
        ] {
            assert!(touches_r15(&bad), "{bad:?}");
            let mut out = vec![Uop::new(k::MOV_IMM, 0x0FFC)];
            out.push(bad);
            assert_eq!(seal(&mut out, 1, &insn, Lowered::Done), Lowered::Generic);
            assert_eq!(out.len(), 2);
            assert_eq!(out[1], Uop::generic(0xE280_0001, 0x1000, 0));
        }
    }

    #[test]
    fn clean_uops_pass_the_backstop_and_get_their_predicate_wrap() {
        let mut ok = Uop::new(k::LDR_IMM, 0x1000);
        ok.rs = 15; // memory flags b0..b3 all set: not a register
        ok.rd = 3;
        ok.rn = 14;
        assert!(!touches_r15(&ok));
        let mut stm = Uop::new(k::STM, 0x1000);
        stm.imm = 0x4010;
        assert!(!touches_r15(&stm));
        let mut pair = Uop::new(k::LDRD, 0x1000);
        pair.rd = 12;
        assert!(!touches_r15(&pair));
        assert!(
            !touches_r15(&Uop::exit(0, 0x1000)),
            "control uops are exempt"
        );
        let insn = at(0x1000, 0x1280_0001); // addne r0,r0,#1
        let mut out = vec![ok];
        assert_eq!(seal(&mut out, 0, &insn, Lowered::Done), Lowered::Done);
        assert_eq!((out.len(), out[0].kind), (2, k::PRED));
    }
}
