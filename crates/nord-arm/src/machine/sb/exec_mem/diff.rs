//! Harness of the differential tests of the memory arms: ARM word encoders, [`uop_of`] (this
//! file's statement of the field table of design note 3.1: the uop a lowerer is specified to emit
//! for a word, or `None` for a form that stays `GENERIC`), the interpreter's handler of a word,
//! and [`diff_case`], which runs one case through both on equal states and logging buses and
//! compares the registers, the ordered access log with pcs, and the bus pc.
//!
//! The random suites and the mutation check are `fuzz.rs`, the hand-computed cases `cases.rs`.

use super::*;
use crate::cpu::Cpu;
use crate::machine::sb::testkit::{CpuSnap, LogBus};

pub(super) type Log = Vec<(u8, u32, u32, u32)>;

/// The pc the candidate's bus starts with: an arm that does not publish its pc shows it in the log.
const NO_PC: u32 = 0xFFFF_FFFF;

/// P, U and W bits of a transfer instruction.
#[derive(Clone, Copy, Debug)]
pub(super) struct Mode {
    pub p: u32,
    pub u: u32,
    pub w: u32,
}

pub(super) const fn mode(p: u32, u: u32, w: u32) -> Mode {
    Mode { p, u, w }
}

/// The offset operand of an encoded transfer.
#[derive(Clone, Copy, Debug)]
pub(super) enum Off {
    Imm(u32),
    Reg(u32),
    Shifted { rm: u32, ty: u32, amount: u32 },
}

const AL: u32 = 0xE000_0000;

/// `ldr`/`ldrb`/`str`/`strb` (always-condition).
pub(super) fn enc_single(l: u32, byte: u32, m: Mode, rn: u32, rd: u32, off: Off) -> u32 {
    let (reg, low) = match off {
        Off::Imm(i) => (0, i & 0xFFF),
        Off::Reg(rm) => (1, rm),
        Off::Shifted { rm, ty, amount } => (1, amount << 7 | ty << 5 | rm),
    };
    AL | 1 << 26
        | reg << 25
        | m.p << 24
        | m.u << 23
        | byte << 22
        | m.w << 21
        | l << 20
        | rn << 16
        | rd << 12
        | low
}

/// `ldrh`/`ldrsb`/`ldrsh`/`strh`/`ldrd`/`strd`: `(l, sh)` is `(1, 1..=3)` for the loads and
/// `(0, 1)`, `(0, 2)`, `(0, 3)` for `strh`, `ldrd`, `strd`.
pub(super) fn enc_half(l: u32, sh: u32, m: Mode, rn: u32, rd: u32, off: Off) -> u32 {
    let (imm, low) = match off {
        Off::Imm(i) => (1, (i & 0xF0) << 4 | i & 0xF),
        Off::Reg(rm) | Off::Shifted { rm, .. } => (0, rm),
    };
    AL | m.p << 24
        | m.u << 23
        | imm << 22
        | m.w << 21
        | l << 20
        | rn << 16
        | rd << 12
        | 0x90
        | sh << 5
        | low
}

/// `ldm`/`stm` with the S bit clear.
pub(super) fn enc_block(l: u32, m: Mode, rn: u32, list: u32) -> u32 {
    AL | 1 << 27 | m.p << 24 | m.u << 23 | m.w << 21 | l << 20 | rn << 16 | list
}

/// The uop of the field table for `word` at `pc`, or `None` when no lowerer emits one: `r15` as
/// base, destination or offset register, an `ldrd`/`strd` pair reaching `r15`, RRX offsets, `S`
/// bit, an empty list or `r15` in it, and non-memory words.
pub(super) fn uop_of(word: u32, pc: u32) -> Option<Uop> {
    let (rn, rd, rm) = (word >> 16 & 15, word >> 12 & 15, word & 15);
    let (p, up, w, l) = (
        word >> 24 & 1,
        word >> 23 & 1,
        word >> 21 & 1,
        word >> 20 & 1 == 1,
    );
    let mut u = Uop::new(0, pc);
    (u.rn, u.rd) = (rn as u8, rd as u8);
    let wb = u8::from(p == 0 || w == 1);
    u.rs = up as u8 | (p as u8) << 1 | wb << 2;
    match word >> 25 & 7 {
        2 | 3 => {
            let (byte, reg) = (word >> 22 & 1 == 1, word >> 25 & 1 == 1);
            if rn == 15 || rd == 15 {
                return None;
            }
            if reg {
                let (ty, amount) = (word >> 5 & 3, word >> 7 & 31);
                if word >> 4 & 1 != 0 || rm == 15 || (ty == 3 && amount == 0) {
                    return None;
                }
                (u.rm, u.sh) = (rm as u8, (ty << 5 | amount) as u8);
            } else {
                u.imm = word & 0xFFF;
            }
            u.kind = match (l, byte, reg) {
                (true, false, false) => k::LDR_IMM,
                (true, true, false) => k::LDRB_IMM,
                (false, false, false) => k::STR_IMM,
                (false, true, false) => k::STRB_IMM,
                (true, false, true) => k::LDR_REG,
                (true, true, true) => k::LDRB_REG,
                (false, false, true) => k::STR_REG,
                (false, true, true) => k::STRB_REG,
            };
        }
        0 if word >> 7 & 1 == 1 && word >> 4 & 1 == 1 && word >> 5 & 3 != 0 => {
            if rn == 15 || rd == 15 {
                return None;
            }
            u.kind = match (l, word >> 5 & 3) {
                (true, 1) => k::LDRH,
                (true, 2) => k::LDRSB,
                (true, _) => k::LDRSH,
                (false, 1) => k::STRH,
                (false, 2) => k::LDRD,
                (false, _) => k::STRD,
            };
            if matches!(u.kind, k::LDRD | k::STRD) && rd >= 14 {
                return None;
            }
            if word >> 22 & 1 == 1 {
                u.imm = word >> 4 & 0xF0 | word & 0xF;
            } else if rm == 15 {
                return None;
            } else {
                (u.rs, u.rm) = (u.rs | F_REG, rm as u8);
            }
        }
        4 => {
            let list = word & 0xFFFF;
            if word >> 22 & 1 == 1 || list == 0 || list >> 15 != 0 || rn == 15 {
                return None;
            }
            u.rd = 0;
            u.kind = if l { k::LDM } else { k::STM };
            u.rs = up as u8 | (p as u8) << 1 | (w as u8) << 2; // WB is the W bit alone
            (u.sh, u.imm) = (list.count_ones() as u8, list);
        }
        _ => return None,
    }
    Some(u)
}

/// What the interpreter did on a case: state before and after, and its access log.
pub(super) struct Ran {
    pub before: [u32; 16],
    pub after: Cpu,
    pub log: Log,
}

/// The interpreter's handler for `word` on a clone of `cpu`; `bus.pc` is the instruction's.
fn interpret(word: u32, pc: u32, cpu: &Cpu, bus: &mut LogBus) -> Cpu {
    let mut c = cpu.clone();
    bus.pc = pc;
    match word >> 25 & 7 {
        4 => c.arm_ldm(bus, word),
        2 | 3 => c.ldst_exec(bus, word, word >> 20 & 1 != 0, word >> 22 & 1 != 0),
        _ => c
            .arm_ldst_half(bus, pc, word)
            .expect("a defined halfword form"),
    }
    c
}

/// Candidate under test: the registers, a bus, a uop.
pub(super) type Exec<'a> = &'a dyn Fn(&mut [u32; 16], &mut LogBus, &Uop);

/// Run `word` through the interpreter and `u` through `exec` on equal states and buses; `Err`
/// names the first difference in registers (and every other `Cpu` field), the ordered access log
/// with pcs, or the final bus pc.
pub(super) fn diff_case(
    word: u32,
    u: &Uop,
    cpu: &Cpu,
    fill: u32,
    exec: Exec,
) -> Result<Ran, String> {
    let (mut ba, mut bb) = (LogBus::new(fill), LogBus::new(fill));
    let a = interpret(word, u.pc, cpu, &mut ba);
    let mut b = cpu.clone();
    bb.pc = NO_PC;
    exec(&mut b.r, &mut bb, u);
    let head = format!("word {word:#010x} at {:#x} (kind {})", u.pc, u.kind);
    if let Some(d) = CpuSnap::of(&a, None).diff(&CpuSnap::of(&b, None)) {
        return Err(format!("{head}: {d}"));
    }
    if ba.log != bb.log {
        return Err(format!(
            "{head}: access logs differ\n interpreter: {:x?}\n arm:         {:x?}",
            ba.log, bb.log
        ));
    }
    if ba.pc != bb.pc {
        return Err(format!("{head}: bus pc {:#x} != {:#x}", ba.pc, bb.pc));
    }
    Ok(Ran {
        before: cpu.r,
        after: a,
        log: ba.log,
    })
}

/// The arms under test.
pub(super) fn real(r: &mut [u32; 16], bus: &mut LogBus, u: &Uop) {
    exec_mem(r, bus, u);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testkit::{random_cpu, Rng};

    fn uop(word: u32) -> Uop {
        uop_of(word, 0xC000_0040).expect("a form the lowerers emit")
    }

    #[test]
    fn the_field_table_of_word_and_byte_transfers() {
        let ldr = enc_single(1, 0, mode(1, 1, 0), 1, 0, Off::Imm(4));
        assert_eq!(ldr, 0xE591_0004, "ldr r0,[r1,#4]");
        let u = uop(ldr);
        assert_eq!(
            (u.kind, u.rd, u.rn, u.rs, u.imm),
            (k::LDR_IMM, 0, 1, F_U | F_P, 4)
        );
        let strb = enc_single(0, 1, mode(0, 0, 0), 3, 2, Off::Imm(1));
        assert_eq!(strb, 0xE443_2001, "strb r2,[r3],#-1");
        let u = uop(strb);
        assert_eq!(
            (u.kind, u.rs, u.imm),
            (k::STRB_IMM, F_WB, 1),
            "post-index writes back"
        );
        let shifted = enc_single(
            1,
            0,
            mode(1, 1, 1),
            1,
            0,
            Off::Shifted {
                rm: 2,
                ty: 1,
                amount: 5,
            },
        );
        assert_eq!(shifted, 0xE7B1_02A2, "ldr r0,[r1,r2,lsr #5]!");
        let u = uop(shifted);
        assert_eq!(
            (u.kind, u.rm, u.sh, u.rs),
            (k::LDR_REG, 2, 1 << 5 | 5, F_U | F_P | F_WB)
        );
    }

    #[test]
    fn the_field_table_of_half_double_and_block_transfers() {
        let ldrh = enc_half(1, 1, mode(1, 1, 0), 1, 0, Off::Imm(0x12));
        assert_eq!(ldrh, 0xE1D1_01B2, "ldrh r0,[r1,#0x12]");
        let u = uop(ldrh);
        assert_eq!((u.kind, u.rs, u.imm), (k::LDRH, F_U | F_P, 0x12));
        let ldrd = enc_half(0, 2, mode(1, 1, 0), 5, 4, Off::Reg(6));
        assert_eq!(ldrd, 0xE185_40D6, "ldrd r4,[r5,r6]");
        let u = uop(ldrd);
        assert_eq!((u.kind, u.rm, u.rs), (k::LDRD, 6, F_U | F_P | F_REG));
        let pop = enc_block(1, mode(1, 0, 1), 13, 0x4030);
        assert_eq!(pop, 0xE93D_4030, "ldmdb sp!,{{r4,r5,lr}}");
        let u = uop(pop);
        assert_eq!(
            (u.kind, u.rn, u.rs, u.sh, u.imm),
            (k::LDM, 13, F_P | F_WB, 3, 0x4030)
        );
        let plain = uop(enc_block(1, mode(0, 1, 0), 0, 0b10));
        assert_eq!(
            plain.rs, F_U,
            "ldmia r0,{{r1}} does not write back, unlike a post-indexed ldr"
        );
    }

    #[test]
    fn forms_that_stay_generic_have_no_uop() {
        let at = |w: u32| uop_of(w, 0x1000);
        assert!(
            at(enc_single(1, 0, mode(1, 1, 0), 15, 0, Off::Imm(0))).is_none(),
            "pc base"
        );
        assert!(
            at(enc_single(1, 0, mode(1, 1, 0), 1, 15, Off::Imm(0))).is_none(),
            "ldr pc"
        );
        assert!(
            at(enc_single(0, 0, mode(1, 1, 0), 1, 15, Off::Imm(0))).is_none(),
            "str pc"
        );
        assert!(
            at(enc_single(1, 0, mode(1, 1, 0), 1, 0, Off::Reg(15))).is_none(),
            "pc offset"
        );
        let rrx = Off::Shifted {
            rm: 2,
            ty: 3,
            amount: 0,
        };
        assert!(
            at(enc_single(1, 0, mode(1, 1, 0), 1, 0, rrx)).is_none(),
            "RRX needs the carry"
        );
        assert!(
            at(enc_half(0, 2, mode(1, 1, 0), 1, 14, Off::Imm(0))).is_none(),
            "ldrd r14 pair"
        );
        assert!(
            at(enc_block(1, mode(0, 1, 0), 0, 0)).is_none(),
            "empty list"
        );
        assert!(
            at(enc_block(1, mode(0, 1, 0), 0, 1 << 15)).is_none(),
            "ldm pc"
        );
        assert!(
            at(enc_block(0, mode(0, 1, 0), 0, 3) | 1 << 22).is_none(),
            "S bit"
        );
        assert!(at(0xE3A0_0001).is_none(), "mov is not a memory word");
    }

    #[test]
    fn the_comparator_catches_a_wrong_register_a_wrong_access_and_a_wrong_pc() {
        let word = enc_block(1, mode(0, 1, 1), 2, 0b1010);
        let u = uop(word);
        let cpu = random_cpu(&mut Rng::new(3), 0xC000_0040);
        assert!(diff_case(word, &u, &cpu, 9, &real).is_ok());
        let wrong_reg = |r: &mut [u32; 16], b: &mut LogBus, u: &Uop| {
            exec_mem(r, b, u);
            r[1] ^= 1;
        };
        let wrong_order = |r: &mut [u32; 16], b: &mut LogBus, u: &Uop| {
            exec_mem(r, b, u);
            b.log.reverse();
        };
        let wrong_pc = |r: &mut [u32; 16], b: &mut LogBus, u: &Uop| {
            exec_mem(r, b, u);
            b.pc ^= 4;
        };
        let wrong: [(&str, Exec); 3] = [
            ("register", &wrong_reg),
            ("order", &wrong_order),
            ("pc", &wrong_pc),
        ];
        for (name, bad) in wrong {
            assert!(
                diff_case(word, &u, &cpu, 9, bad).is_err(),
                "{name} not caught"
            );
        }
    }
}
