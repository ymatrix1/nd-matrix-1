//! ARM multiplies (32 and 64 bit), saturating add/subtract and the ARMv5E 16-bit multiplies.

use super::alu::{qadd, qsub};
use super::Cpu;

fn half(v: u32, top: bool) -> i32 {
    (if top { v >> 16 } else { v & 0xFFFF }) as u16 as i16 as i32
}

impl Cpu {
    pub(crate) fn arm_mul(&mut self, insn: u32) {
        let hi = (insn >> 16 & 0xF) as usize;
        let lo = (insn >> 12 & 0xF) as usize;
        let rs = self.r[(insn >> 8 & 0xF) as usize];
        let rm = self.r[(insn & 0xF) as usize];
        let s = insn >> 20 & 1 != 0;
        match insn >> 21 & 0xF {
            0 | 1 => {
                let acc = if insn >> 21 & 1 != 0 { self.r[lo] } else { 0 };
                let v = rm.wrapping_mul(rs).wrapping_add(acc);
                self.r[hi] = v;
                if s {
                    self.set_nz(v);
                }
            }
            op => {
                let signed = op & 2 != 0;
                let accumulate = op & 1 != 0;
                let prod: u64 = if signed {
                    (rm as i32 as i64).wrapping_mul(rs as i32 as i64) as u64
                } else {
                    (rm as u64) * (rs as u64)
                };
                let acc = if accumulate {
                    (self.r[hi] as u64) << 32 | self.r[lo] as u64
                } else {
                    0
                };
                let v = prod.wrapping_add(acc);
                self.r[lo] = v as u32;
                self.r[hi] = (v >> 32) as u32;
                if s {
                    self.n = v >> 63 != 0;
                    self.z = v == 0;
                }
            }
        }
    }

    /// QADD, QSUB, QDADD, QDSUB.
    pub(crate) fn arm_qaddsub(&mut self, insn: u32) {
        let rd = (insn >> 12 & 0xF) as usize;
        let rn = self.r[(insn >> 16 & 0xF) as usize] as i32;
        let rm = self.r[(insn & 0xF) as usize] as i32;
        let op = insn >> 21 & 3;
        let (rn, sat0) = if op & 2 != 0 {
            qadd(rn, rn)
        } else {
            (rn, false)
        };
        let (v, sat1) = if op & 1 == 0 {
            qadd(rm, rn)
        } else {
            qsub(rm, rn)
        };
        self.q |= sat0 | sat1;
        self.r[rd] = v as u32;
    }

    /// SMLAxy, SMLAWy / SMULWy, SMLALxy, SMULxy.
    pub(crate) fn arm_dsp_mul(&mut self, insn: u32) {
        let hi = (insn >> 16 & 0xF) as usize;
        let lo = (insn >> 12 & 0xF) as usize;
        let rs = self.r[(insn >> 8 & 0xF) as usize];
        let rm = self.r[(insn & 0xF) as usize];
        let x = insn >> 5 & 1 != 0;
        let y = insn >> 6 & 1 != 0;
        match insn >> 21 & 3 {
            0 => {
                let p = half(rm, x) * half(rs, y);
                let acc = self.r[lo] as i32;
                let (v, ov) = p.overflowing_add(acc);
                self.q |= ov;
                self.r[hi] = v as u32;
            }
            1 => {
                let p = ((rm as i32 as i64) * half(rs, y) as i64) >> 16;
                if x {
                    self.r[hi] = p as u32;
                } else {
                    let (v, ov) = (p as i32).overflowing_add(self.r[lo] as i32);
                    self.q |= ov;
                    self.r[hi] = v as u32;
                }
            }
            2 => {
                let p = (half(rm, x) * half(rs, y)) as i64;
                let acc = ((self.r[hi] as u64) << 32 | self.r[lo] as u64) as i64;
                let v = acc.wrapping_add(p) as u64;
                self.r[lo] = v as u32;
                self.r[hi] = (v >> 32) as u32;
            }
            _ => self.r[hi] = (half(rm, x) * half(rs, y)) as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::testkit::run_arm;

    #[test]
    fn mul_mla_and_long_multiplies() {
        // mov r0,#7 ; mov r1,#6 ; mul r2,r0,r1 ; mla r3,r0,r1,r2 ; mvn r4,#0 ; umull r5,r6,r4,r4 ;
        // smull r7,r8,r4,r4
        let code = [
            0xE3A0_0007,
            0xE3A0_1006,
            0xE002_0190,
            0xE023_2190,
            0xE3E0_4000,
            0xE086_5494,
            0xE0C8_7494,
        ];
        let (c, _) = run_arm(&code, 7);
        assert_eq!((c.r[2], c.r[3]), (42, 84));
        assert_eq!((c.r[5], c.r[6]), (1, 0xFFFF_FFFE));
        assert_eq!((c.r[7], c.r[8]), (1, 0));
    }

    #[test]
    fn qadd_saturates_and_sets_sticky_q() {
        // mvn r0,#0x80000000 -> 0x7FFFFFFF ; mov r1,#1 ; qadd r2,r1,r0
        let code = [0xE3E0_0102, 0xE3A0_1001, 0xE100_2051];
        let (c, _) = run_arm(&code, 3);
        assert_eq!(c.r[2], 0x7FFF_FFFF);
        assert!(c.q);
    }

    #[test]
    fn smulbb_and_smultt_use_the_selected_halves() {
        // mvn r0,#1 (0xFFFFFFFE) ; mov r1,#4 ; orr r1,r1,#0x40000 ; smulbb r2,r0,r1 ; smultt r3,r0,r1
        let code = [
            0xE3E0_0001,
            0xE3A0_1004,
            0xE381_1701,
            0xE162_0180,
            0xE163_01E0,
        ];
        let (c, _) = run_arm(&code, 5);
        assert_eq!(c.r[2] as i32, -8);
        assert_eq!(c.r[3] as i32, -4);
    }
}
