//! OWNER: W1-VGEN. The constructs of the structured L1 programs (design note 8, L1): counted
//! loops, `bl` and `bx lr` pairs, conditional runs, literal loads, stores into the own page, the
//! next word and a literal the code loaded, device accesses, Thumb entry and status register
//! moves. [`Gen`] appends them to one program body; `program.rs` wraps the body in a layout.
//!
//! The register contract is in `parts.rs`; every construct keeps it, so any sequence of
//! constructs is a program that stays in its memory map and whose loops end.

use super::asm::{self as a, AL};
use super::parts::{straight_word, with_cond, CONDS};
use super::program::{FAR_AT, FUNC_AT, FUNC_IDX, LIT_AT};
use super::CODE;
use crate::machine::sb::testkit::Rng;

mod effects;

/// The state of one program body being generated.
pub(super) struct Gen<'a> {
    pub rng: &'a mut Rng,
    /// The body, from `CODE`.
    pub w: Vec<u32>,
    /// Indices of the plain straight words a patch store may overwrite.
    pub patchable: Vec<usize>,
    /// Indices of the `str r3,[r10,#0]` words whose offset is chosen once the body is complete.
    pub fixups: Vec<usize>,
    /// Loop nesting: 0 at top level; the loop at depth `d` counts in `r11 + d`.
    pub depth: u32,
}

impl<'a> Gen<'a> {
    pub fn new(rng: &'a mut Rng) -> Self {
        Self {
            rng,
            w: Vec::new(),
            patchable: Vec::new(),
            fixups: Vec::new(),
            depth: 0,
        }
    }

    /// Address of the next word to emit.
    pub fn at(&self) -> u32 {
        CODE + 4 * self.w.len() as u32
    }

    fn push(&mut self, w: u32) {
        self.w.push(w);
    }

    /// Address of word `idx` of the in-page pool.
    fn lit(idx: usize) -> u32 {
        LIT_AT + 4 * idx as u32
    }

    /// `ldr rd, [pc, #x]` of pool word `idx`.
    fn load_lit(&mut self, rd: u32, idx: usize) {
        let at = self.at();
        self.push(a::mem_pc(true, rd, at, Self::lit(idx)));
    }

    /// `b<cond>` from here to `target`.
    pub fn jump(&mut self, cond: u32, target: u32) {
        let at = self.at();
        self.push(a::branch(cond, false, at, target));
    }

    /// `k` straight words, one in six conditional.
    pub fn straight(&mut self, k: usize) {
        for _ in 0..k {
            let mut w = straight_word(self.rng);
            if self.rng.chance(6) {
                w = with_cond(w, self.rng.pick(&CONDS));
            }
            self.patchable.push(self.w.len());
            self.push(w);
        }
    }

    /// A compare followed by 2 to 7 conditionally executed words that may change the flags
    /// themselves (the predicated runs of `pred.rs`).
    pub fn cond_run(&mut self) {
        let cmp = if self.rng.flag() {
            a::cmp_imm(self.rng.below(8), self.rng.below(8))
        } else {
            a::dp_shift_imm(10, true, self.rng.below(8), 0, self.rng.below(8), 0, 0)
        };
        self.push(cmp);
        for _ in 0..self.rng.range(2, 8) {
            let w = straight_word(self.rng);
            let cond = self.rng.pick(&CONDS);
            self.push(with_cond(w, cond));
        }
    }

    /// `b<cc>` over 1 to 5 straight words: a guard whose both sides run in some case.
    pub fn skip(&mut self) {
        let k = self.rng.range(1, 6) as usize;
        self.straight(1);
        let cmp = a::cmp_imm(self.rng.below(8), self.rng.below(4));
        self.push(cmp);
        let (cond, at) = (self.rng.pick(&CONDS), self.at());
        self.jump(cond, at + 4 + 4 * k as u32);
        self.straight(k);
    }

    /// A counted loop in `r11 + depth`: `mov rc,#n; L: body; subs rc,rc,#1; bne L`, or the
    /// count-up form `add; cmp; blt L`.
    pub fn loop_block(&mut self) {
        let rc = 11 + self.depth;
        let n = self.rng.range(1, 9);
        let up = self.rng.chance(3);
        self.push(a::mov_imm(rc, if up { 0 } else { n }));
        let top = self.at();
        self.depth += 1;
        for _ in 0..self.rng.range(1, 4) {
            if self.depth < 2 && self.rng.chance(6) {
                self.loop_block();
            } else {
                self.simple();
            }
        }
        self.depth -= 1;
        if up {
            self.push(a::add_imm(false, rc, rc, 1));
            self.push(a::cmp_imm(rc, n));
            self.jump(11, top); // blt
        } else {
            self.push(a::sub_imm(true, rc, rc, 1));
            self.jump(1, top); // bne
        }
    }

    /// A call: `bl` to a near or far callee, or `mov lr,pc; bx r4` / `mov lr,pc; mov pc,r4` with
    /// the address loaded from the pool. Callee 5 counts in `r12`: not inside the inner loop.
    pub fn call(&mut self) {
        let near = if self.depth >= 2 { 5 } else { 6 };
        if self.rng.chance(3) {
            let idx = FUNC_IDX + self.rng.below(near) as usize;
            self.load_lit(4, idx);
            self.push(0xE1A0_E00F); // mov lr,pc
            let jump = if self.rng.flag() {
                a::bx(AL, 4)
            } else {
                0xE1A0_F004
            };
            self.push(jump); // bx r4 | mov pc,r4
            return;
        }
        let target = if self.rng.chance(4) {
            self.rng.pick(&FAR_AT)
        } else {
            FUNC_AT[self.rng.below(near) as usize]
        };
        let at = self.at();
        self.push(a::branch(AL, true, at, target));
    }

    /// Pc-relative loads of the in-page pool (word, unaligned, byte) and the pc-relative data
    /// processing forms (`add rd,pc,#x`, `mov rd,pc`, a register-shifted `pc` operand: pc + 12).
    pub fn literal(&mut self) {
        let (at, rd) = (self.at(), self.rng.below(8));
        let target = Self::lit(self.rng.below(16) as usize);
        let w = match self.rng.below(6) {
            0 | 1 => a::mem_pc(true, rd, at, target),
            2 => a::mem_pc(true, rd, at, target + 1 + self.rng.below(3)),
            3 => a::ldst_imm(
                true,
                true,
                true,
                false,
                15,
                rd,
                (target - at - 8) as i32 + 1,
            ),
            4 => a::dp_imm(
                AL,
                self.rng.pick(&[4, 2, 13]),
                false,
                15,
                rd,
                0,
                self.rng.below(0x40) * 4,
            ),
            _ => {
                let (op, ty, rs) = (
                    self.rng.pick(&[13, 4, 12]),
                    self.rng.below(4),
                    self.rng.below(8),
                );
                a::dp_shift_reg(op, false, self.rng.below(8), rd, 15, ty, rs)
            }
        };
        self.push(w);
        self.straight(1);
    }

    /// One construct that is safe at any loop depth.
    pub fn simple(&mut self) {
        match self.rng.below(10) {
            0..=4 => {
                let k = self.rng.range(1, 7) as usize;
                self.straight(k);
            }
            5 => self.cond_run(),
            6 => self.skip(),
            7 => self.call(),
            8 => self.literal(),
            _ => self.indexed(),
        }
    }

    /// One block of any kind the current depth allows.
    pub fn block(&mut self) {
        let top = self.depth < 2;
        match self.rng.below(100) {
            0..=19 => {
                let k = self.rng.range(2, 9) as usize;
                self.straight(k);
            }
            20..=31 if top => self.loop_block(),
            32..=41 => self.call(),
            42..=48 => self.cond_run(),
            49..=55 => self.skip(),
            56..=61 => self.literal(),
            62..=65 => self.patch_own(),
            66..=68 => self.patch_next(),
            69..=72 if top => self.patch_literal(),
            73..=79 => self.device(),
            80..=83 if top => self.thumb(),
            84..=87 => self.psr(),
            88..=92 => self.indexed(),
            93..=95 => self.jump_literal(),
            _ => self.simple(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen(seed: u64, f: impl FnOnce(&mut Gen)) -> Vec<u32> {
        let mut rng = Rng::new(seed);
        let mut g = Gen::new(&mut rng);
        f(&mut g);
        g.w
    }

    /// The word a branch at index `i` of a body targets.
    fn target(words: &[u32], i: usize) -> u32 {
        let off = ((words[i] & 0x00FF_FFFF) << 8) as i32 >> 8;
        (CODE + 4 * i as u32 + 8).wrapping_add((off * 4) as u32)
    }

    #[test]
    fn a_conditional_run_is_a_compare_and_conditional_words() {
        for seed in 0..20 {
            let w = gen(seed, |g| g.cond_run());
            assert!((3..=9).contains(&w.len()), "{} words", w.len());
            assert_eq!(w[0] >> 28, AL);
            assert!(
                w[1..].iter().all(|w| w >> 28 != AL),
                "every word of the run is conditional"
            );
        }
    }

    #[test]
    fn a_loop_starts_with_its_counter_and_branches_back_to_the_first_body_word() {
        for seed in 0..40 {
            let w = gen(seed, |g| g.loop_block());
            let last = w.len() - 1;
            assert_eq!(w[0] >> 20, 0xE3A, "mov rc,#n");
            assert_eq!(w[last] >> 24 & 0xF, 0xA, "ends with a plain branch");
            assert!(w[last] >> 28 == 1 || w[last] >> 28 == 11, "bne or blt");
            assert_eq!(
                target(&w, last),
                CODE + 4,
                "seed {seed}: the back edge goes to the body"
            );
            assert!(w[0] >> 12 & 0xF >= 11, "the counter is r11 or r12");
        }
    }

    #[test]
    fn calls_branch_and_link_or_go_through_a_pool_address() {
        let w = gen(3, |g| {
            for _ in 0..30 {
                g.call();
            }
        });
        assert!(w.iter().any(|w| w >> 24 == 0xEB), "bl");
        assert!(w.contains(&0xE1A0_E00F), "mov lr,pc");
    }
}
