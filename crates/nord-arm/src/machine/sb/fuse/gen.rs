//! OWNER: W2-PEEP (tests). A guest-program generator rich in what the peephole passes look for:
//! a compare or test and the `Bcc` it feeds, adjacent moves of small constants, adjacent loads
//! off one base, runs of conditional instructions under one condition (with a flag writer, a
//! different condition or an unconditional word breaking some of them), and chains of S
//! operations whose flags are overwritten, read by a condition, a carry-in, a status read, a
//! store or a `GENERIC`, or nothing at all.
//!
//! Registers: `r0..r7` data, `r8` a base for transfers. Every word is a well-formed instruction
//! (no undefined word, no fault); the interpreter and the traces see the same words. Branches go
//! forward a few words inside the program, a counted loop goes back to its own head, and the
//! program ends in `b .`.

use crate::machine::sb::testgen::asm::{self as a, AL};
use crate::machine::sb::testgen::CODE;
use crate::machine::sb::testkit::Rng;

/// Replace the condition field of `w`.
fn with_cond(w: u32, cond: u32) -> u32 {
    w & 0x0FFF_FFFF | cond << 28
}

/// Byte address of word `i`.
fn at(i: usize) -> u32 {
    CODE + 4 * i as u32
}

struct Gen<'a> {
    rng: &'a mut Rng,
    w: Vec<u32>,
}

impl Gen<'_> {
    fn reg(&mut self) -> u32 {
        self.rng.below(8)
    }

    /// An immediate operand: no rotation (shifter carry is the old C) or a real rotation.
    fn imm(&mut self) -> (u32, u32) {
        let rot = if self.rng.chance(3) {
            self.rng.range(1, 16)
        } else {
            0
        };
        (rot, self.rng.biased() & 0xFF)
    }

    /// A flag-setting data-processing word of any form, `S` set; compares get `rd = 0`.
    fn s_op(&mut self) -> u32 {
        let op = self.rng.below(16);
        let (rn, rd, rm, rs) = (self.reg(), self.reg(), self.reg(), self.reg());
        let rd = if (8..=11).contains(&op) { 0 } else { rd };
        match self.rng.below(8) {
            0..=2 => {
                let (rot, imm8) = self.imm();
                a::dp_imm(AL, op, true, rn, rd, rot, imm8)
            }
            3..=4 => a::dp_shift_imm(op, true, rn, rd, rm, 0, 0),
            5..=6 => {
                let amount = self.rng.pick(&[0, 1, 5, 31]);
                a::dp_shift_imm(op, true, rn, rd, rm, self.rng.below(4), amount)
            }
            _ => a::dp_shift_reg(op, true, rn, rd, rm, self.rng.below(4), rs),
        }
    }

    /// A data-processing word without `S` (never a compare).
    fn plain_op(&mut self) -> u32 {
        let op = self.rng.pick(&[0u32, 1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 15]);
        let (rn, rd, rm) = (self.reg(), self.reg(), self.reg());
        match self.rng.below(5) {
            0..=2 => {
                let (rot, imm8) = self.imm();
                a::dp_imm(AL, op, false, rn, rd, rot, imm8)
            }
            3 => a::dp_shift_imm(op, false, rn, rd, rm, 0, 0),
            _ => a::dp_shift_imm(op, false, rn, rd, rm, self.rng.below(4), self.rng.below(32)),
        }
    }

    /// Something that reads or observes the flags (or nothing).
    fn reader(&mut self) {
        let (rd, rn) = (self.reg(), self.reg());
        let w = match self.rng.below(9) {
            0 => a::dp_imm(self.rng.below(14), 4, false, rn, rd, 0, 1), // addcc rd, rn, #1
            1 => a::dp_imm(AL, 5, false, rn, rd, 0, 3),                 // adc
            2 => a::dp_shift_imm(6, false, rn, rd, self.reg(), 0, 0),   // sbc
            3 => a::mrs(rd, false),
            4 => a::str(rd, 8, 4 * self.rng.below(8) as i32),
            // umull: GENERIC (`mul` is a native kind since W2-MUL); RdHi, RdLo and Rm are distinct
            5 => 0xE080_0090 | rd << 16 | ((rd + 1) & 7) << 12 | rn << 8 | ((rd + 2) & 7),
            6 => a::ldr(rd, 8, 4 * self.rng.below(8) as i32),
            7 => return self.cmp_branch(),
            _ => a::mov_imm(rd, self.rng.below(200)),
        };
        self.w.push(w);
    }

    fn chain(&mut self) {
        for _ in 0..self.rng.range(2, 5) {
            let w = self.s_op();
            self.w.push(w);
            if self.rng.chance(3) {
                let w = self.plain_op();
                self.w.push(w);
            }
        }
        if !self.rng.chance(5) {
            self.reader();
        }
    }

    /// `cmp`/`cmn`/`tst`/`teq` (immediate or register, sometimes shifted) and a forward `Bcc`.
    fn cmp_branch(&mut self) {
        let (rn, rm) = (self.reg(), self.reg());
        let op = self.rng.pick(&[10u32, 10, 10, 8, 8, 11, 9]);
        let w = match self.rng.below(6) {
            0..=2 => {
                let (rot, imm8) = self.imm();
                a::dp_imm(AL, op, true, rn, 0, rot, imm8)
            }
            3..=4 => a::dp_shift_imm(op, true, rn, 0, rm, 0, 0),
            _ => a::dp_shift_imm(op, true, rn, 0, rm, self.rng.below(4), self.rng.below(32)),
        };
        self.w.push(w);
        if self.rng.chance(6) {
            let w = self.plain_op();
            self.w.push(w); // something between: no fusion
        }
        let here = self.w.len();
        let skip = self.rng.range(1, 5) as usize;
        self.w.push(a::branch(
            self.rng.below(14),
            false,
            at(here),
            at(here + 1 + skip),
        ));
    }

    fn mov_pair(&mut self) {
        for _ in 0..self.rng.range(2, 5) {
            let rd = self.reg();
            let (rot, imm8) = if self.rng.chance(4) {
                (4, 0xFF)
            } else {
                (0, self.rng.below(256))
            };
            let w = a::dp_imm(AL, 13, false, 0, rd, rot, imm8);
            self.w.push(w);
        }
    }

    fn ldr_pair(&mut self) {
        let base = if self.rng.chance(4) { self.reg() } else { 8 };
        for i in 0..self.rng.range(2, 4) {
            let rd = if i == 0 && self.rng.chance(5) {
                base
            } else {
                self.reg()
            };
            let off = 4 * self.rng.below(16) as i32 * if self.rng.chance(6) { -1 } else { 1 };
            let w = match self.rng.below(10) {
                0 => a::ldst_imm(true, false, true, true, base, rd, off), // write-back: stays unfused
                1 => a::ldst_imm(true, false, false, false, base, rd, off.abs()), // post-index
                2 => a::ldst_imm(true, true, true, false, base, rd, off), // ldrb
                _ => a::ldr(rd, base, off),
            };
            self.w.push(w);
        }
    }

    fn cond_run(&mut self) {
        let cond = self.rng.below(14);
        for _ in 0..self.rng.range(2, 11) {
            let w = match self.rng.below(12) {
                0 => self.s_op(),
                1 => a::str(self.reg(), 8, 4 * self.rng.below(8) as i32),
                2 => a::ldr(self.reg(), 8, 4 * self.rng.below(8) as i32),
                3 => a::mrs(self.reg(), false),
                4 => 0xE000_0090 | self.reg() << 16 | self.reg() << 8 | self.reg(),
                _ => self.plain_op(),
            };
            let c = match self.rng.below(14) {
                0 => self.rng.below(14), // a different condition ends the run
                1 => AL,                 // an unconditional word does too
                _ => cond,
            };
            self.w.push(with_cond(w, c));
        }
    }

    fn counted_loop(&mut self) {
        let head = self.w.len() + 1;
        self.w.push(a::mov_imm(11, self.rng.range(1, 4)));
        for _ in 0..self.rng.range(1, 4) {
            let w = if self.rng.flag() {
                self.plain_op()
            } else {
                self.s_op()
            };
            self.w.push(w);
        }
        self.w.push(a::sub_imm(true, 11, 11, 1));
        let here = self.w.len();
        self.w.push(a::branch(1, false, at(here), at(head))); // bne head
    }
}

/// A program of about `n` words (and a tail of padding and `b .`).
pub(super) fn program(rng: &mut Rng, n: usize) -> Vec<u32> {
    let mut g = Gen { rng, w: Vec::new() };
    while g.w.len() < n {
        match g.rng.below(100) {
            0..=19 => g.cmp_branch(),
            20..=27 => g.mov_pair(),
            28..=37 => g.ldr_pair(),
            38..=52 => g.cond_run(),
            53..=77 => g.chain(),
            78..=81 => g.counted_loop(),
            82..=89 => {
                let w = g.plain_op();
                g.w.push(w);
            }
            _ => {
                let w = g.s_op();
                g.w.push(w);
            }
        }
    }
    // forward branches may land up to 6 words past the last construct
    g.w.extend([a::NOP; 8]);
    let here = g.w.len();
    g.w.push(a::branch(AL, false, at(here), at(here)));
    g.w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::trace::{build, BuildOpts};
    use crate::machine::sb::uop::k;
    use crate::mem::Mem;

    #[test]
    fn a_program_is_deterministic_and_ends_in_a_spin() {
        let (a, b) = (program(&mut Rng::new(3), 60), program(&mut Rng::new(3), 60));
        assert_eq!(a, b);
        assert!(a.len() >= 68);
        assert_eq!(*a.last().unwrap(), 0xEAFF_FFFE, "b .");
        assert_ne!(a, program(&mut Rng::new(4), 60));
    }

    #[test]
    fn programs_hold_the_shapes_the_passes_look_for() {
        let on = BuildOpts {
            max_uops: 128,
            ..BuildOpts::default()
        };
        let mut kinds = [0u32; 256];
        let mut rng = Rng::new(0x6E4);
        for _ in 0..40 {
            let words = program(&mut rng, 120);
            let mut mem = Mem::new();
            for (i, w) in words.iter().enumerate() {
                mem.write32(at(i), *w);
            }
            for start in (0..words.len()).step_by(3) {
                if let Some(sb) = build(&mut mem, at(start), &on) {
                    for u in sb.uops.iter() {
                        kinds[u.kind as usize] += 1;
                    }
                }
            }
        }
        for (name, kind) in [
            ("cmp imm", k::CMP_IMM),
            ("cmp reg", k::CMP_REG),
            ("tst imm", k::TST_IMM),
            ("guard", k::GUARD),
            ("pred", k::PRED),
            ("mov imm", k::MOV_IMM),
            ("ldr imm", k::LDR_IMM),
            ("str imm", k::STR_IMM),
            ("generic", k::GENERIC),
            ("loop back", k::LOOP_BACK),
        ] {
            assert!(
                kinds[kind as usize] > 20,
                "{name}: {}",
                kinds[kind as usize]
            );
        }
        let s_ops = (0..96u8)
            .filter(|id| id & 1 == 1)
            .map(|id| kinds[id as usize])
            .sum::<u32>();
        assert!(s_ops > 200, "generic S operations: {s_ops}");
    }
}
