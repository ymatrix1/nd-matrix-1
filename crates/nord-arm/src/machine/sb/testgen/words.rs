//! OWNER: W1-VGEN. One random instruction word per instruction family, with the operand
//! distributions the lowering and executor tests need (design note 8, L0: "all 96 DP ids x
//! conditions, shift amounts, every P/U/W combination, `rn == rd`, `ldm/stm` over a page end").
//!
//! [`random_of`] draws a word of one [`Family`]; the condition is "always" three times in four.
//! Registers come from `r0..r14` with `r15` drawn one time in sixteen (the `pc` rules of the
//! lowerers), `rd == rn` one time in eight, and every barrel-shifter and addressing-mode field
//! takes all its values. Every word of a family decodes to the class its doc names (a test checks
//! it), so a lowering test can ask for "300 random words of this class" without filtering.

mod family;
mod pick;

use crate::machine::sb::testkit::Rng;
pub(crate) use family::Family;
use pick::{amount, branch_off, half_transfer, rd_for, reg, reg_list, reg_np, word_transfer};

/// One random word of `family` (see the module docs for the distributions).
pub(crate) fn random_of(rng: &mut Rng, family: Family) -> u32 {
    let cond = rng.cond() << 28;
    match family {
        Family::DpImm | Family::DpShiftImm | Family::DpShiftReg => {
            let op = rng.below(16);
            let s = if (8..=11).contains(&op) {
                1
            } else {
                rng.below(2)
            };
            let (rn, rm) = (reg(rng), reg(rng));
            let rd = rd_for(rng, rn);
            let head = op << 21 | s << 20 | rn << 16 | rd << 12;
            let operand = match family {
                Family::DpImm => 1 << 25 | rng.below(16) << 8 | rng.biased() & 0xFF,
                Family::DpShiftImm => amount(rng) << 7 | rng.below(4) << 5 | rm,
                _ => reg_np(rng) << 8 | rng.below(4) << 5 | 1 << 4 | rm,
            };
            cond | head | operand
        }
        Family::Mul => {
            let (a, s) = (rng.below(2), rng.below(2));
            cond | a << 21
                | s << 20
                | reg(rng) << 16
                | reg_np(rng) << 12
                | reg_np(rng) << 8
                | 0x90
                | reg(rng)
        }
        Family::MulLong => {
            let (u, a, s) = (rng.below(2), rng.below(2), rng.below(2));
            cond | 0x0080_0090
                | u << 22
                | a << 21
                | s << 20
                | reg_np(rng) << 16
                | reg_np(rng) << 12
                | reg_np(rng) << 8
                | reg_np(rng)
        }
        Family::Clz => cond | 0x016F_0F10 | reg(rng) << 12 | reg(rng),
        Family::Qadd => {
            cond | 0x0100_0050 | rng.below(4) << 21 | reg(rng) << 16 | reg(rng) << 12 | reg(rng)
        }
        Family::DspMul => {
            cond | 0x0100_0080
                | rng.below(4) << 21
                | rng.below(2) << 6
                | rng.below(2) << 5
                | reg_np(rng) << 16
                | reg_np(rng) << 12
                | reg_np(rng) << 8
                | reg_np(rng)
        }
        Family::Ldr => cond | word_transfer(rng, true),
        Family::Str => cond | word_transfer(rng, false),
        Family::Half => {
            let load = !rng.chance(4);
            let sh = if load { rng.range(1, 4) } else { 1 };
            let rn = reg(rng);
            let rd = rd_for(rng, rn);
            cond | half_transfer(rng, load, sh, rd, rn)
        }
        Family::Double => {
            let rn = reg(rng);
            let rd = if rng.chance(8) {
                reg(rng)
            } else {
                rng.below(7) * 2
            };
            let sh = rng.range(2, 4);
            cond | half_transfer(rng, false, sh, rd, rn)
        }
        Family::Ldm | Family::Stm => {
            let rn = reg(rng);
            let list = reg_list(rng, rn);
            let s = u32::from(rng.chance(16));
            cond | 0x0800_0000
                | rng.below(2) << 24
                | rng.below(2) << 23
                | s << 22
                | rng.below(2) << 21
                | u32::from(family == Family::Ldm) << 20
                | rn << 16
                | list
        }
        Family::Swp => {
            cond | 0x0100_0090 | rng.below(2) << 22 | reg(rng) << 16 | reg(rng) << 12 | reg(rng)
        }
        Family::B => cond | 0x0A00_0000 | branch_off(rng),
        Family::Bl => cond | 0x0B00_0000 | branch_off(rng),
        Family::Bx => cond | 0x012F_FF10 | reg(rng),
        Family::BlxReg => cond | 0x012F_FF30 | reg(rng),
        Family::BlxImm => 0xF000_0000 | 0x0A00_0000 | rng.below(2) << 24 | branch_off(rng),
        Family::Mrs => cond | 0x010F_0000 | rng.below(2) << 22 | reg(rng) << 12,
        Family::Msr => {
            let (spsr, mask) = (rng.below(2) << 22, rng.below(16) << 16);
            if rng.flag() {
                cond | 0x0320_F000 | spsr | mask | rng.below(16) << 8 | rng.biased() & 0xFF
            } else {
                cond | 0x0120_F000 | spsr | mask | reg(rng)
            }
        }
        Family::Cp15 => {
            let crn = rng.pick(&[0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 13]);
            let (crm, op2, load) = (rng.below(16), rng.below(8), rng.below(2));
            let wfi = rng.chance(8);
            let (crn, crm, op2) = if wfi { (7, 0, 4) } else { (crn, crm, op2) };
            cond | 0x0E00_0F10 | load << 20 | crn << 16 | reg(rng) << 12 | op2 << 5 | crm
        }
        Family::Swi => cond | 0x0F00_0000 | rng.next() & 0x00FF_FFFF,
        Family::Undefined => match rng.below(4) {
            0 => cond | 0x0700_0010 | rng.next() & 0x00FF_FF00 | rng.below(16), // media space
            1 => cond | 0x0C00_0000 | rng.next() & 0x01FF_FFFF,                 // ldc/stc
            2 => cond | 0x0E00_0010 | rng.below(15) << 8 & 0xE00 | rng.next() & 0x00EF_F0EF, // coprocessors other than p15
            _ => loop {
                // condition field 15 that is neither `blx <label>` nor `pld`
                let w = 0xF000_0000 | rng.next() & 0x0FFF_FFFF;
                if w >> 25 & 7 != 5 && w & 0xFD70_F000 != 0xF550_F000 {
                    break w;
                }
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::predecode::{decode_arm, op};
    use crate::cpu::{Cpu, Fault};
    use crate::machine::sb::testkit::{edge_cpu, oracle_trace, DiffRun, Need};
    use std::collections::BTreeSet;

    /// The predecode classes (and, for data processing, the operand form) each family's words
    /// must fall in.
    fn classes(f: Family) -> &'static [u8] {
        match f {
            Family::DpImm | Family::DpShiftImm | Family::DpShiftReg => &[],
            Family::Mul | Family::MulLong => &[op::MUL],
            Family::Clz
            | Family::Qadd
            | Family::DspMul
            | Family::Swp
            | Family::Mrs
            | Family::Msr
            | Family::BlxReg
            | Family::Cp15
            | Family::Swi => &[op::SLOW],
            Family::Ldr => &[op::LDR, op::LDRB],
            Family::Str => &[op::STR, op::STRB],
            Family::Half | Family::Double => &[op::HALF],
            Family::Ldm | Family::Stm => &[op::LDM],
            Family::B => &[op::B],
            Family::Bl => &[op::BL],
            Family::Bx => &[op::BX],
            Family::BlxImm => &[op::UNCOND],
            Family::Undefined => &[op::SLOW, op::UNCOND],
        }
    }

    #[test]
    fn every_word_of_a_family_decodes_to_the_class_the_family_names() {
        for f in Family::ALL {
            let mut rng = Rng::new(f as u64 + 100);
            for _ in 0..300 {
                let w = random_of(&mut rng, f);
                let e = decode_arm(w);
                match f {
                    Family::DpImm | Family::DpShiftImm | Family::DpShiftReg => {
                        let form = Cpu::dp_form(w);
                        assert!(e.op >= op::DP_BASE, "{f:?} {w:#010x} class {}", e.op);
                        assert_eq!(form, f as u32, "{f:?} {w:#010x} operand form");
                    }
                    _ => assert!(classes(f).contains(&e.op), "{f:?} {w:#010x} class {}", e.op),
                }
            }
        }
    }

    #[test]
    fn words_are_deterministic_per_seed_and_the_families_draw_distinct_words() {
        for f in Family::ALL {
            let a: Vec<u32> = (0..50).map(|_| random_of(&mut Rng::new(5), f)).collect();
            assert!(a.windows(2).all(|w| w[0] == w[1]), "same seed, same word");
            let mut rng = Rng::new(6);
            let words: BTreeSet<u32> = (0..200).map(|_| random_of(&mut rng, f)).collect();
            let small = matches!(
                f,
                Family::Bx | Family::BlxReg | Family::Mrs | Family::Clz | Family::BlxImm
            );
            let min = if small {
                10
            } else if matches!(f, Family::B | Family::Bl) {
                50
            } else {
                100
            };
            assert!(
                words.len() > min,
                "{f:?} draws only {} distinct words of 200",
                words.len()
            );
        }
    }

    #[test]
    fn the_dp_families_cover_all_opcodes_forms_conditions_and_the_pc_and_rd_equals_rn() {
        let mut rng = Rng::new(7);
        let mut ids = BTreeSet::new();
        let (mut conds, mut pc_rn, mut rd_rn, mut s_cmp) = (BTreeSet::new(), false, false, true);
        for f in [Family::DpImm, Family::DpShiftImm, Family::DpShiftReg] {
            for _ in 0..3000 {
                let w = random_of(&mut rng, f);
                ids.insert(decode_arm(w).op);
                conds.insert(w >> 28);
                pc_rn |= w >> 16 & 15 == 15;
                rd_rn |= w >> 16 & 15 == w >> 12 & 15;
                s_cmp &= !(8..=11).contains(&(w >> 21 & 15)) || w >> 20 & 1 == 1;
            }
        }
        // 3 forms x 16 opcodes x S = 96 ids, minus the 12 compares without S: those words are the
        // miscellaneous space (`mrs`, `msr`, `bx`, `clz`, ...), not data processing
        assert_eq!(ids.len(), 84, "every reachable data-processing id");
        assert!((0..=14).all(|c| conds.contains(&c)) && !conds.contains(&15));
        assert!(pc_rn && rd_rn && s_cmp);
    }

    /// Each family does what it is for on the interpreter, counted on the oracle: loads load,
    /// stores store, branches jump, flag-setting words change flags, undefined words fault.
    #[test]
    fn the_families_exercise_what_they_are_for_on_the_interpreter() {
        let want = |f: Family| -> Need {
            let n = Need {
                cases: 400,
                max_idle_pct: 60,
                ..Need::default()
            };
            match f {
                Family::Ldr | Family::Half => Need {
                    loads: 100,
                    reg_writes: 100,
                    ..n
                },
                Family::Ldm => Need { loads: 100, ..n },
                Family::Str | Family::Stm | Family::Double => Need { stores: 100, ..n },
                Family::Swp => Need {
                    loads: 100,
                    stores: 100,
                    ..n
                },
                Family::B | Family::Bl | Family::Bx | Family::BlxReg | Family::BlxImm => {
                    Need { jumps: 100, ..n }
                }
                Family::DpImm | Family::DpShiftImm | Family::DpShiftReg => Need {
                    reg_writes: 150,
                    flag_changes: 60,
                    ..n
                },
                Family::Mul
                | Family::MulLong
                | Family::Clz
                | Family::Qadd
                | Family::DspMul
                | Family::Mrs => Need {
                    reg_writes: 150,
                    ..n
                },
                Family::Msr => Need {
                    flag_changes: 30,
                    max_idle_pct: 100,
                    ..n
                },
                Family::Cp15 => Need {
                    reg_writes: 20,
                    faults: 5,
                    max_idle_pct: 100,
                    ..n
                },
                Family::Swi | Family::Undefined => Need { faults: 280, ..n },
            }
        };
        for f in Family::ALL {
            let mut rng = Rng::new(f as u64 + 1000);
            let mut run = DiffRun::default();
            for case in 0..400 {
                let cpu = edge_cpu(&mut rng, 0x1000);
                let w = random_of(&mut rng, f);
                let t = oracle_trace(&cpu, &[w], case, 1);
                if f.is_fault_free() {
                    assert_eq!(t.fault, None, "{f:?} {w:#010x} faulted");
                }
                run.note(&cpu, &t);
            }
            run.assert_exercised(&format!("{f:?}"), &want(f));
        }
    }

    #[test]
    fn the_fault_families_fault_with_the_fault_their_name_says() {
        let mut rng = Rng::new(11);
        let cpu = edge_cpu(&mut rng, 0x1000);
        let cond_al = |w: u32| w & 0x0FFF_FFFF | 0xE000_0000;
        for _ in 0..200 {
            let w = random_of(&mut rng, Family::Swi);
            let t = oracle_trace(&cpu, &[cond_al(w)], 1, 1);
            assert!(matches!(t.fault, Some(Fault::Swi { .. })), "{w:#x}");
            let w = random_of(&mut rng, Family::Undefined);
            let w = if w >> 28 == 15 { w } else { cond_al(w) };
            let t = oracle_trace(&cpu, &[w], 1, 1);
            assert!(matches!(t.fault, Some(Fault::Undefined { .. })), "{w:#x}");
        }
        let wfi = (0..400).any(|_| {
            let w = cond_al(random_of(&mut rng, Family::Cp15));
            oracle_trace(&cpu, &[w], 1, 1).fault == Some(Fault::Wfi)
        });
        assert!(wfi, "the cp15 family reaches wfi");
    }
}
