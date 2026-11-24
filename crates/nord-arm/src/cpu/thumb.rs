//! Thumb state decoder: shifts, arithmetic, ALU, high-register ops and branches.

use super::alu::{add_c, shift};
use super::{Bus, Cpu, Fault};

impl Cpu {
    fn nzcv_add(&mut self, a: u32, b: u32, c: bool) -> u32 {
        let (r, cy, v) = add_c(a, b, c);
        self.set_nz(r);
        self.c = cy;
        self.v = v;
        r
    }

    fn thumb_alu(&mut self, op: u32, rd: usize, rs: usize) {
        let a = self.r[rd];
        let b = self.r[rs];
        let carry = self.c;
        match op {
            0 | 8 | 12 | 14 | 1 => {
                let r = match op {
                    0 | 8 => a & b,
                    1 => a ^ b,
                    12 => a | b,
                    _ => a & !b,
                };
                self.set_nz(r);
                if op != 8 {
                    self.r[rd] = r;
                }
            }
            2..=4 | 7 => {
                let ty = match op {
                    2 => 0,
                    3 => 1,
                    4 => 2,
                    _ => 3,
                };
                let (r, c) = shift(ty, b, a, carry, false);
                self.set_nz(r);
                self.c = c;
                self.r[rd] = r;
            }
            5 => self.r[rd] = self.nzcv_add(a, b, carry),
            6 => self.r[rd] = self.nzcv_add(a, !b, carry),
            9 => self.r[rd] = self.nzcv_add(0, !b, true),
            10 => {
                self.nzcv_add(a, !b, true);
            }
            11 => {
                self.nzcv_add(a, b, false);
            }
            13 => {
                let r = a.wrapping_mul(b);
                self.set_nz(r);
                self.r[rd] = r;
            }
            _ => {
                let r = !b;
                self.set_nz(r);
                self.r[rd] = r;
            }
        }
    }

    pub(crate) fn exec_thumb<B: Bus>(
        &mut self,
        bus: &mut B,
        pc: u32,
        insn: u32,
    ) -> Result<(), Fault> {
        let rd = (insn & 7) as usize;
        let rs = (insn >> 3 & 7) as usize;
        match insn >> 11 {
            0..=2 => {
                let (r, c) = shift(insn >> 11, insn >> 6 & 0x1F, self.r[rs], self.c, true);
                self.set_nz(r);
                self.c = c;
                self.r[rd] = r;
            }
            3 => {
                let rn = insn >> 6 & 7;
                let b = if insn >> 10 & 1 != 0 {
                    rn
                } else {
                    self.r[rn as usize]
                };
                let a = self.r[rs];
                self.r[rd] = if insn >> 9 & 1 != 0 {
                    self.nzcv_add(a, !b, true)
                } else {
                    self.nzcv_add(a, b, false)
                };
            }
            op @ 4..=7 => {
                let rd = (insn >> 8 & 7) as usize;
                let imm = insn & 0xFF;
                match op {
                    4 => {
                        self.set_nz(imm);
                        self.r[rd] = imm;
                    }
                    5 => {
                        self.nzcv_add(self.r[rd], !imm, true);
                    }
                    6 => self.r[rd] = self.nzcv_add(self.r[rd], imm, false),
                    _ => self.r[rd] = self.nzcv_add(self.r[rd], !imm, true),
                }
            }
            8 => {
                if insn >> 10 & 1 == 0 {
                    self.thumb_alu(insn >> 6 & 0xF, rd, rs);
                } else {
                    return self.thumb_hi(pc, insn);
                }
            }
            9 => {
                let rd = (insn >> 8 & 7) as usize;
                let a = (self.r[15] & !3).wrapping_add((insn & 0xFF) << 2);
                self.r[rd] = bus.read32(a);
            }
            10..=19 => return self.thumb_ldst(bus, pc, insn),
            20 | 21 => {
                let rd = (insn >> 8 & 7) as usize;
                let base = if insn >> 11 & 1 == 0 {
                    self.r[15] & !3
                } else {
                    self.r[13]
                };
                self.r[rd] = base.wrapping_add((insn & 0xFF) << 2);
            }
            22 | 23 => return self.thumb_misc(bus, pc, insn),
            24 | 25 => self.thumb_ldm_stm(bus, insn),
            26 | 27 => {
                let cond = insn >> 8 & 0xF;
                if cond == 15 {
                    return Err(Fault::Swi {
                        pc,
                        imm: insn & 0xFF,
                    });
                }
                if cond == 14 {
                    return Err(Fault::Undefined { pc, insn });
                }
                if self.cond(cond) {
                    let off = ((insn & 0xFF) as i8 as i32 as u32) << 1;
                    self.branch(self.r[15].wrapping_add(off), false);
                }
            }
            28 => {
                let off = (((insn & 0x7FF) << 21) as i32 >> 20) as u32;
                self.branch(self.r[15].wrapping_add(off), false);
            }
            30 => {
                let off = ((((insn & 0x7FF) << 21) as i32) >> 9) as u32;
                self.r[14] = self.r[15].wrapping_add(off);
            }
            29 | 31 => {
                // BLX suffix (29), BL suffix (31)
                if insn >> 11 == 29 && insn & 1 != 0 {
                    return Err(Fault::Undefined { pc, insn });
                }
                let target = self.r[14].wrapping_add((insn & 0x7FF) << 1);
                self.r[14] = pc.wrapping_add(2) | 1;
                if insn >> 11 == 29 {
                    self.thumb = false;
                    self.branch(target & !3, false);
                } else {
                    self.branch(target, false);
                }
            }
            _ => return Err(Fault::Undefined { pc, insn }),
        }
        Ok(())
    }

    fn thumb_hi(&mut self, pc: u32, insn: u32) -> Result<(), Fault> {
        let rd = ((insn & 7) | (insn >> 4 & 8)) as usize;
        let rs = (insn >> 3 & 0xF) as usize;
        let b = self.r[rs];
        match insn >> 8 & 3 {
            0 => {
                let v = self.r[rd].wrapping_add(b);
                if rd == 15 {
                    self.branch(v, false);
                } else {
                    self.r[rd] = v;
                }
            }
            1 => {
                self.nzcv_add(self.r[rd], !b, true);
            }
            2 => {
                if rd == 15 {
                    self.branch(b, false);
                } else {
                    self.r[rd] = b;
                }
            }
            _ => {
                if insn >> 7 & 1 != 0 {
                    self.r[14] = pc.wrapping_add(2) | 1;
                }
                self.branch(b, true);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::testkit::run_thumb;

    #[test]
    fn immediate_arithmetic_and_flags() {
        // movs r0,#5 ; subs r1,r0,#5 ; adds r2,r0,#3 ; cmp r0,#6
        let (c, _) = run_thumb(&[0x2005, 0x1F41, 0x1CC2, 0x2806], 4);
        assert_eq!((c.r[0], c.r[2]), (5, 8));
        assert!(c.n && !c.c);
    }

    #[test]
    fn alu_register_ops_and_shifts() {
        // movs r0,#12 ; movs r1,#2 ; lsls r0,r1 (0x4088) ; mvns r2,r1 ; muls r1,r0
        let (c, _) = run_thumb(&[0x200C, 0x2102, 0x4088, 0x43CA, 0x4341], 5);
        assert_eq!((c.r[0], c.r[2], c.r[1]), (48, !2u32, 96));
    }

    #[test]
    fn branches_and_bl_pair() {
        // 1000: bl +0x10 (pair) ; 1004: movs r1,#9 ; 1006: b . ; 1014: movs r0,#4 ; bx lr
        let code: Vec<u16> = {
            let mut v = vec![0u16; 16];
            v[0] = 0xF000; // prefix, high offset 0
            v[1] = 0xF80A; // suffix, low offset 10 << 1 = 0x14 -> 0x1004 + 0x14 = 0x1018
            v[2] = 0x2109;
            v[3] = 0xE7FE;
            v[12] = 0x2004;
            v[13] = 0x4770;
            v
        };
        // target = 0x1004 + 0x14 = 0x1018 = index 12
        let (c, _) = run_thumb(&code, 4);
        assert_eq!(c.r[0], 4);
        assert_eq!(c.r[15], 0x1004);
        assert_eq!(c.r[14] & 1, 1);
    }

    #[test]
    fn hi_register_mov_add_and_bx() {
        // movs r0,#1 ; mov r8,r0 (0x4680) ; add r8,r8 (0x44C0) ; mov r1,r8 (0x4641)
        let (c, _) = run_thumb(&[0x2001, 0x4680, 0x44C0, 0x4641], 4);
        assert_eq!(c.r[1], 2);
    }
}
