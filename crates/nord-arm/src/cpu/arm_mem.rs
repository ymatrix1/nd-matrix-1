//! ARM load/store instructions: word/byte, halfword/doubleword, multiple, swap.

use super::alu::shift;
use super::{Bus, Cpu, Fault};

impl Cpu {
    /// Load a word with the pre-ARMv6 unaligned rule (aligned read, rotated).
    #[inline(always)]
    pub(crate) fn load32<B: Bus>(bus: &mut B, addr: u32) -> u32 {
        bus.read32(addr & !3).rotate_right(8 * (addr & 3))
    }

    /// Write back a loaded value; a load into r15 interworks (ARMv5T).
    #[inline(always)]
    pub(crate) fn set_loaded(&mut self, rd: usize, v: u32) {
        if rd == 15 {
            self.branch(v, true);
        } else {
            self.r[rd] = v;
        }
    }

    pub(crate) fn arm_ldst<B: Bus>(&mut self, bus: &mut B, insn: u32) {
        self.ldst_exec(bus, insn, insn >> 20 & 1 != 0, insn >> 22 & 1 != 0);
    }

    /// Single word/byte load (`l`) or store with the byte flag `byte`; the predecoded handlers
    /// pass constants for both.
    #[inline(always)]
    pub(crate) fn ldst_exec<B: Bus>(&mut self, bus: &mut B, insn: u32, l: bool, byte: bool) {
        let (p, u, w) = (
            insn >> 24 & 1 != 0,
            insn >> 23 & 1 != 0,
            insn >> 21 & 1 != 0,
        );
        let rn = (insn >> 16 & 0xF) as usize;
        let rd = (insn >> 12 & 0xF) as usize;
        let off = if insn >> 25 & 1 == 0 {
            insn & 0xFFF
        } else {
            shift(
                insn >> 5 & 3,
                insn >> 7 & 0x1F,
                self.r[(insn & 0xF) as usize],
                self.c,
                true,
            )
            .0
        };
        let base = self.r[rn];
        let moved = if u {
            base.wrapping_add(off)
        } else {
            base.wrapping_sub(off)
        };
        let addr = if p { moved } else { base };
        let wb = !p || w;
        if l {
            let v = if byte {
                bus.read8(addr) as u32
            } else {
                Self::load32(bus, addr)
            };
            if wb && rn != rd {
                self.r[rn] = moved;
            }
            self.set_loaded(rd, v);
        } else {
            let v = self.r[rd];
            if byte {
                bus.write8(addr, v as u8);
            } else {
                bus.write32(addr & !3, v);
            }
            if wb {
                self.r[rn] = moved;
            }
        }
    }

    pub(crate) fn arm_ldst_half<B: Bus>(
        &mut self,
        bus: &mut B,
        pc: u32,
        insn: u32,
    ) -> Result<(), Fault> {
        let (p, u, imm, w, l) = (
            insn >> 24 & 1 != 0,
            insn >> 23 & 1 != 0,
            insn >> 22 & 1 != 0,
            insn >> 21 & 1 != 0,
            insn >> 20 & 1 != 0,
        );
        let rn = (insn >> 16 & 0xF) as usize;
        let rd = (insn >> 12 & 0xF) as usize;
        let sh = insn >> 5 & 3;
        let off = if imm {
            (insn >> 4 & 0xF0) | (insn & 0xF)
        } else {
            self.r[(insn & 0xF) as usize]
        };
        let base = self.r[rn];
        let moved = if u {
            base.wrapping_add(off)
        } else {
            base.wrapping_sub(off)
        };
        let addr = if p { moved } else { base };
        let wb = !p || w;
        match (l, sh) {
            (true, 1) => {
                let v = bus.read16(addr & !1) as u32;
                self.finish_load(rn, rd, wb, moved, v);
            }
            (true, 2) => {
                let v = bus.read8(addr) as i8 as i32 as u32;
                self.finish_load(rn, rd, wb, moved, v);
            }
            (true, 3) => {
                let v = bus.read16(addr & !1) as i16 as i32 as u32;
                self.finish_load(rn, rd, wb, moved, v);
            }
            (false, 1) => {
                bus.write16(addr & !1, self.r[rd] as u16);
                if wb {
                    self.r[rn] = moved;
                }
            }
            (false, 2) => {
                // LDRD
                let lo = bus.read32(addr & !3);
                let hi = bus.read32((addr & !3).wrapping_add(4));
                if wb {
                    self.r[rn] = moved;
                }
                self.r[rd] = lo;
                self.r[(rd + 1) & 15] = hi;
            }
            (false, 3) => {
                // STRD
                bus.write32(addr & !3, self.r[rd]);
                bus.write32((addr & !3).wrapping_add(4), self.r[(rd + 1) & 15]);
                if wb {
                    self.r[rn] = moved;
                }
            }
            _ => return Err(Fault::Undefined { pc, insn }),
        }
        Ok(())
    }

    fn finish_load(&mut self, rn: usize, rd: usize, wb: bool, moved: u32, v: u32) {
        if wb && rn != rd {
            self.r[rn] = moved;
        }
        self.set_loaded(rd, v);
    }

    #[allow(clippy::needless_range_loop)] // register-list bit i selects register i
    pub(crate) fn arm_ldm<B: Bus>(&mut self, bus: &mut B, insn: u32) {
        let (p, u, s, w, l) = (
            insn >> 24 & 1 != 0,
            insn >> 23 & 1 != 0,
            insn >> 22 & 1 != 0,
            insn >> 21 & 1 != 0,
            insn >> 20 & 1 != 0,
        );
        let rn = (insn >> 16 & 0xF) as usize;
        let list = insn & 0xFFFF;
        let n = list.count_ones();
        let base = self.r[rn];
        let low = match (u, p) {
            (true, false) => base,
            (true, true) => base.wrapping_add(4),
            (false, false) => base.wrapping_sub(4 * n).wrapping_add(4),
            (false, true) => base.wrapping_sub(4 * n),
        };
        let wb_val = if u {
            base.wrapping_add(4 * n)
        } else {
            base.wrapping_sub(4 * n)
        };
        let mut addr = low;
        if l {
            let mut pc_val = None;
            for i in 0..16usize {
                if list >> i & 1 != 0 {
                    let v = bus.read32(addr & !3);
                    addr = addr.wrapping_add(4);
                    if i == 15 {
                        pc_val = Some(v);
                    } else {
                        self.r[i] = v;
                    }
                }
            }
            if w && list >> rn & 1 == 0 {
                self.r[rn] = wb_val;
            }
            if let Some(v) = pc_val {
                if s {
                    let sp = self.spsr;
                    self.set_cpsr(sp);
                    self.branch(v, false);
                } else {
                    self.branch(v, true);
                }
            }
        } else {
            let first = list.trailing_zeros() as usize;
            for i in 0..16usize {
                if list >> i & 1 != 0 {
                    bus.write32(addr & !3, self.r[i]);
                    addr = addr.wrapping_add(4);
                    if w && i == first {
                        self.r[rn] = wb_val;
                    }
                }
            }
            if w && list == 0 {
                self.r[rn] = wb_val;
            }
        }
    }

    pub(crate) fn arm_swp<B: Bus>(&mut self, bus: &mut B, insn: u32) {
        let rn = (insn >> 16 & 0xF) as usize;
        let rd = (insn >> 12 & 0xF) as usize;
        let rm = (insn & 0xF) as usize;
        let addr = self.r[rn];
        let new = self.r[rm];
        if insn >> 22 & 1 != 0 {
            let old = bus.read8(addr) as u32;
            bus.write8(addr, new as u8);
            self.r[rd] = old;
        } else {
            let old = Self::load32(bus, addr);
            bus.write32(addr & !3, new);
            self.r[rd] = old;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::bus::Bus;
    use crate::cpu::testkit::run_arm;

    #[test]
    fn str_ldr_pre_post_index_and_unaligned_rotate() {
        // mov r0,#0x2000 ; mov r1,#0xAB ; str r1,[r0,#4]! ; ldr r2,[r0],#8 ; ldr r3,[r0,#-7]
        let code = [
            0xE3A0_0A02,
            0xE3A0_10AB,
            0xE5A0_1004,
            0xE490_2008,
            0xE510_3007,
        ];
        let (c, mut b) = run_arm(&code, 5);
        assert_eq!(b.read32(0x2004), 0xAB);
        assert_eq!(c.r[2], 0xAB);
        assert_eq!(c.r[0], 0x200C);
        // r3 = word at 0x2004 (aligned down from 0x2005) rotated right by 8
        assert_eq!(c.r[3], b.read32(0x2004).rotate_right(8));
    }

    #[test]
    fn ldm_stm_full_descending_stack() {
        // mov r0,#1 ; mov r1,#2 ; mov sp,#0x3000 ; stmfd sp!,{r0,r1} ; mov r0,#0 ; ldmfd sp!,{r2,r3}
        let code = [
            0xE3A0_0001,
            0xE3A0_1002,
            0xE3A0_DA03,
            0xE92D_0003,
            0xE3A0_0000,
            0xE8BD_000C,
        ];
        let (c, mut b) = run_arm(&code, 6);
        assert_eq!((c.r[2], c.r[3], c.r[13]), (1, 2, 0x3000));
        assert_eq!(b.read32(0x2FF8), 1);
    }

    #[test]
    fn halfword_signed_and_doubleword() {
        // mov r0,#0x2000 ; mvn r1,#0 ; strh r1,[r0] ; ldrsh r2,[r0] ; ldrh r3,[r0]
        let code = [
            0xE3A0_0A02,
            0xE3E0_1000,
            0xE1C0_10B0,
            0xE1D0_20F0,
            0xE1D0_30B0,
        ];
        let (c, _) = run_arm(&code, 5);
        assert_eq!((c.r[2], c.r[3]), (0xFFFF_FFFF, 0xFFFF));
    }

    #[test]
    fn ldrd_strd_pair_and_swp() {
        // mov r0,#0x2000 ; mov r2,#5 ; mov r3,#6 ; strd r2,[r0] ; ldrd r4,[r0] ; swp r6,r2,[r0]
        let code = [
            0xE3A0_0A02,
            0xE3A0_2005,
            0xE3A0_3006,
            0xE1C0_20F0,
            0xE1C0_40D0,
            0xE100_6092,
        ];
        let (c, mut b) = run_arm(&code, 6);
        assert_eq!((c.r[4], c.r[5], c.r[6]), (5, 6, 5));
        assert_eq!(b.read32(0x2000), 5);
    }
}
