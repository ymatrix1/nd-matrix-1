//! OWNER: W1-VGEN. The constructs of `blocks.rs` that write memory the program runs from or talk to
//! devices: stores into the own page, the next word and a literal the code loads, device accesses,
//! Thumb entry and status register moves.

use super::super::asm::{self as a, thumb, AL};
use super::super::program::{DEV_IDX, LIT_AT, PATCH_IDX};
use super::Gen;

/// Entries of `DEV` (see `program.rs`) that lie in a poll window: a poll loop on them ends.
const POLL_DEVICES: u32 = 8;
/// Index in `DEV` of the first of two fake DSP ports that complete a message with two stores.
const PORT_DEVICE: usize = 14;

impl<'a> Gen<'a> {
    /// Store a valid instruction word over a straight word of the program (`str r3,[r10,#4n]`,
    /// `n` chosen once the body is complete): a store into the executing page.
    pub fn patch_own(&mut self) {
        let k = self.rng.below(16) as usize;
        self.load_lit(3, PATCH_IDX + k);
        self.fixups.push(self.w.len());
        self.push(a::str(3, 10, 0));
    }

    /// Store a valid word over the very next instruction, then run it.
    pub fn patch_next(&mut self) {
        let k = self.rng.below(16) as usize;
        self.load_lit(3, PATCH_IDX + k);
        self.push(a::ldst_imm(false, false, true, false, 15, 3, -4)); // str r3,[pc,#-4]
        self.push(0xE1B0_0001); // movs r0,r1: replaced before it runs
    }

    /// A loop that rewrites a literal it loads (a folded literal changes under the trace).
    pub fn patch_literal(&mut self) {
        let rc = 11 + self.depth;
        let target = Self::lit(self.rng.below(8) as usize);
        let (r0, r5) = (self.rng.below(3), self.rng.range(3, 6));
        let trips = self.rng.range(2, 6);
        self.push(a::mov_imm(rc, trips));
        let top = self.at();
        self.load_lit(r0, ((target - LIT_AT) / 4) as usize);
        self.push(a::dp_shift_imm(4, false, 2, 2, r0, 0, 0)); // add r2,r2,r<r0>
        let step = self.rng.range(1, 200);
        self.push(a::add_imm(false, r5, r5, step));
        let at = self.at();
        self.push(a::mem_pc(false, r5, at, target));
        self.push(a::sub_imm(true, rc, rc, 1));
        self.jump(1, top);
    }

    /// Device accesses through an address loaded from the pool: a load, a store and load back,
    /// two stores to a DSP port (one message), a byte load and halfword store, or a poll loop.
    pub fn device(&mut self) {
        match self.rng.below(5) {
            0 => {
                let d = self.rng.below(16) as usize;
                self.load_lit(5, DEV_IDX + d);
                self.push(a::ldr(6, 5, 0));
            }
            1 => {
                let (d, r) = (self.rng.below(16) as usize, self.rng.below(5));
                self.load_lit(5, DEV_IDX + d);
                self.push(a::str(r, 5, 0));
                self.push(a::ldr(6, 5, 0));
            }
            2 => {
                let (d, r, s) = (
                    self.rng.below(2) as usize,
                    self.rng.below(5),
                    self.rng.below(5),
                );
                self.load_lit(5, DEV_IDX + PORT_DEVICE + d);
                self.push(a::str(r, 5, 0));
                self.push(a::str(s, 5, 0));
            }
            3 => {
                let d = self.rng.below(16) as usize;
                self.load_lit(5, DEV_IDX + d);
                self.push(a::ldst_imm(true, true, true, false, 5, 6, 1));
                self.push(0xE1C5_60B0); // strh r6,[r5]
            }
            _ => {
                let (d, mask) = (
                    self.rng.below(POLL_DEVICES) as usize,
                    self.rng.pick(&[1, 2, 0x10, 0x80]),
                );
                self.load_lit(5, DEV_IDX + d);
                let top = self.at();
                self.push(a::ldr(6, 5, 0));
                self.push(a::dp_imm(AL, 8, true, 6, 0, 0, mask)); // tst
                self.jump(0, top); // beq L
            }
        }
    }

    /// Enter Thumb state, run 4 to 12 Thumb instructions on `r0..r7` and the stack, and return to
    /// ARM state with `bx lr` (`lr` holds the word after the block).
    pub fn thumb(&mut self) {
        let n = self.rng.range(2, 7) as usize; // words of Thumb code
        self.push(a::dp_imm(AL, 4, false, 15, 14, 0, 4 + 4 * n as u32)); // add lr,pc,#..
        self.push(a::dp_imm(AL, 12, false, 15, 12, 0, 1)); // orr r12,pc,#1
        self.push(a::bx(AL, 12));
        let mut hw: Vec<u16> = Vec::new();
        for _ in 0..2 * n - 1 {
            let (rd, rm, imm) = (self.rng.below(8), self.rng.below(8), self.rng.below(256));
            hw.push(match self.rng.below(8) {
                0 => thumb::mov_imm(rd, self.rng.biased() & 0xFF),
                1 => thumb::add_imm(rd, imm),
                2 => thumb::sub_imm(rd, imm),
                3 => thumb::cmp_imm(rd, imm),
                4 => thumb::shift(self.rng.flag(), rd, rm, self.rng.below(32)),
                5 => thumb::add_reg(rd, rm, self.rng.below(8)),
                6 => thumb::sp_rel(false, rd, self.rng.below(16)),
                _ => thumb::sp_rel(true, rd, self.rng.below(16)),
            });
        }
        hw.push(thumb::BX_LR);
        self.w.extend(thumb::pack(&hw));
    }

    /// Status register moves, and a short visit to another mode that returns to SVC.
    pub fn psr(&mut self) {
        if self.rng.chance(3) {
            let mode = self.rng.pick(&[0x11, 0x12, 0x17, 0x1B, 0x1F, 0x10]);
            self.push(a::msr_imm(false, 1, 0, mode));
            let k = self.rng.range(1, 4) as usize;
            self.straight(k);
            self.push(a::msr_imm(false, 1, 0, 0x13));
            return;
        }
        for _ in 0..self.rng.range(1, 4) {
            let rd = self.rng.below(8);
            let w = match self.rng.below(4) {
                0 => a::mrs(rd, false),
                1 => a::mrs(rd, true),
                2 => a::msr_imm(false, 8, self.rng.below(16), self.rng.below(256)),
                _ => a::msr_imm(true, 8, self.rng.below(16), self.rng.below(256)),
            };
            self.push(w);
        }
    }

    /// A bounded register offset: `and r7,rX,#0xFC; ldr/str rY,[r8,r7]` (and the byte forms).
    pub fn indexed(&mut self) {
        let x = self.rng.below(7);
        self.push(a::dp_imm(AL, 0, false, x, 7, 0, 0xFC));
        let (load, byte, rd) = (self.rng.flag(), self.rng.chance(3), self.rng.below(7));
        self.push(0xE788_0007 | (byte as u32) << 22 | (load as u32) << 20 | rd << 12);
    }

    /// `ldr pc,[pc,#-4]` over an inline address word: a jump through a literal in the stream.
    pub fn jump_literal(&mut self) {
        let at = self.at();
        self.push(a::ldst_imm(true, false, true, false, 15, 15, -4));
        self.push(at + 8);
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::CODE;
    use super::*;
    use crate::machine::sb::testkit::{oracle_trace, random_cpu, Rng};

    fn gen(seed: u64, f: impl FnOnce(&mut Gen)) -> Vec<u32> {
        let mut rng = Rng::new(seed);
        let mut g = Gen::new(&mut rng);
        f(&mut g);
        g.w
    }

    #[test]
    fn the_thumb_block_enters_runs_and_leaves_thumb_state() {
        let mut words = gen(24, |g| {
            g.thumb();
            g.straight(1);
        });
        let end = CODE + 4 * (words.len() as u32 - 1);
        words.push(0xEAFF_FFFE);
        let mut cpu = random_cpu(&mut Rng::new(1), CODE);
        (cpu.r[13], cpu.r[8]) = (0x9000_2000, 0x9000_0000);
        let mut saw_thumb = false;
        for steps in 1..=40u64 {
            let t = oracle_trace(&cpu, &words, 5, steps);
            saw_thumb |= t.cpu.thumb;
            assert_eq!(t.fault, None, "step {steps}");
            if t.cpu.r[15] == end {
                assert!(
                    !t.cpu.thumb && saw_thumb,
                    "ARM again, after a visit to Thumb state"
                );
                return;
            }
        }
        panic!("the Thumb block never returned to the word after it");
    }

    #[test]
    fn device_blocks_load_their_address_from_the_pool() {
        let d = gen(4, |g| {
            for _ in 0..30 {
                g.device();
            }
        });
        assert!(
            d.iter().any(|w| w & 0x0F7F_0000 == 0x051F_0000),
            "pc-relative load of a device address"
        );
        assert!(d.contains(&0xE1C5_60B0), "strh r6,[r5]");
    }

    #[test]
    fn the_patch_blocks_register_a_fixup_or_overwrite_the_next_word() {
        let mut rng = Rng::new(9);
        let mut g = Gen::new(&mut rng);
        g.straight(2);
        g.patch_own();
        assert_eq!((g.fixups.len(), g.w.len()), (1, 4));
        g.patch_next();
        assert_eq!(
            g.w[g.w.len() - 2] & 0xFFFF_F000,
            0xE50F_3000,
            "str r3,[pc,#-4]"
        );
        assert_eq!(
            g.patchable.len(),
            2,
            "only straight words are patch targets"
        );
    }

    #[test]
    fn status_and_indexed_blocks_are_bounded_and_the_literal_jump_goes_two_words_on() {
        let w = gen(5, |g| {
            g.psr();
            g.indexed();
            g.jump_literal();
        });
        let n = w.len();
        assert_eq!(
            w[n - 1],
            CODE + 4 * (n as u32 - 2) + 8,
            "the inline address word"
        );
        assert_eq!(w[n - 2], 0xE51F_F004, "ldr pc,[pc,#-4]");
        assert_eq!(w[n - 4] >> 20, 0xE20, "and r7,rX,#0xFC");
    }
}
