//! ARM data-processing instructions.

use super::alu::{add_c, arm_imm, shift};
use super::Cpu;

impl Cpu {
    /// Second operand of a data-processing instruction with its shifter carry.
    pub(crate) fn dp_operand(&self, insn: u32) -> (u32, bool) {
        if insn >> 25 & 1 != 0 {
            let v = arm_imm(insn & 0xFF, insn >> 8 & 0xF);
            let c = if insn >> 8 & 0xF == 0 {
                self.c
            } else {
                v >> 31 != 0
            };
            return (v, c);
        }
        let rm = (insn & 0xF) as usize;
        let ty = insn >> 5 & 3;
        if insn >> 4 & 1 != 0 {
            // register-specified shift: an r15 operand reads pc + 12
            let val = if rm == 15 {
                self.r[15].wrapping_add(4)
            } else {
                self.r[rm]
            };
            let amt = self.r[(insn >> 8 & 0xF) as usize];
            shift(ty, amt, val, self.c, false)
        } else {
            shift(ty, insn >> 7 & 0x1F, self.r[rm], self.c, true)
        }
    }

    pub(crate) fn arm_dp(&mut self, _pc: u32, insn: u32) {
        let op = insn >> 21 & 0xF;
        let s = insn >> 20 & 1 != 0;
        let rd = (insn >> 12 & 0xF) as usize;
        let rn = (insn >> 16 & 0xF) as usize;
        let (b, sc) = self.dp_operand(insn);
        let a = if rn == 15 && insn >> 25 & 1 == 0 && insn >> 4 & 1 != 0 {
            self.r[15].wrapping_add(4)
        } else {
            self.r[rn]
        };
        let c = self.c;
        // (result, carry, overflow) for arithmetic; logic ops keep the shifter carry
        let (res, carry, ovf, arith) = match op {
            0 | 8 => (a & b, sc, self.v, false),
            1 | 9 => (a ^ b, sc, self.v, false),
            2 | 10 => {
                let (r, c, v) = add_c(a, !b, true);
                (r, c, v, true)
            }
            3 => {
                let (r, c, v) = add_c(b, !a, true);
                (r, c, v, true)
            }
            4 | 11 => {
                let (r, c, v) = add_c(a, b, false);
                (r, c, v, true)
            }
            5 => {
                let (r, c, v) = add_c(a, b, c);
                (r, c, v, true)
            }
            6 => {
                let (r, c, v) = add_c(a, !b, c);
                (r, c, v, true)
            }
            7 => {
                let (r, c, v) = add_c(b, !a, c);
                (r, c, v, true)
            }
            12 => (a | b, sc, self.v, false),
            13 => (b, sc, self.v, false),
            14 => (a & !b, sc, self.v, false),
            _ => (!b, sc, self.v, false),
        };
        let test = (8..=11).contains(&op);
        if s {
            if rd == 15 && !test {
                // movs pc, lr style exception return: restore CPSR from SPSR
                let sp = self.spsr;
                self.set_cpsr(sp);
            } else {
                self.set_nz(res);
                self.c = carry;
                if arith {
                    self.v = ovf;
                }
            }
        }
        if !test {
            if rd == 15 {
                self.branch(res, false);
            } else {
                self.r[rd] = res;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::testkit::run_arm;

    #[test]
    fn subs_sets_carry_as_not_borrow_and_zero() {
        // mov r0,#5 ; subs r1,r0,#5 ; subs r2,r0,#6
        let (c, _) = run_arm(&[0xE3A0_0005, 0xE250_1005, 0xE250_2006], 2);
        assert!(c.z && c.c && !c.n && !c.v);
        let (c, _) = run_arm(&[0xE3A0_0005, 0xE250_1005, 0xE250_2006], 3);
        assert!(c.n && !c.c && !c.z);
        assert_eq!(c.r[2], 0xFFFF_FFFF);
    }

    #[test]
    fn adc_sbc_chain_64_bit_add() {
        // r0:r1 = 0xFFFFFFFF:0 ; r2:r3 = 1:0 ; adds r4,r0,r2 ; adc r5,r1,r3
        let code = [
            0xE3E0_0000, // mvn r0,#0
            0xE3A0_1000, // mov r1,#0
            0xE3A0_2001, // mov r2,#1
            0xE3A0_3000, // mov r3,#0
            0xE090_4002, // adds r4,r0,r2
            0xE0A1_5003, // adc r5,r1,r3
        ];
        let (c, _) = run_arm(&code, 6);
        assert_eq!((c.r[4], c.r[5]), (0, 1));
    }

    #[test]
    fn shifted_register_operand_and_mvn() {
        // mov r0,#3 ; mov r1,r0,lsl #4 ; mvn r2,r1 ; movs r3,r0,lsr #1 (carry out = 1)
        let (c, _) = run_arm(&[0xE3A0_0003, 0xE1A0_1200, 0xE1E0_2001, 0xE1B0_30A0], 4);
        assert_eq!((c.r[1], c.r[2], c.r[3], c.c), (0x30, !0x30u32, 1, true));
    }

    #[test]
    fn pc_operand_reads_eight_ahead_and_write_branches() {
        // add r0,pc,#0 ; add pc,pc,#4 ; (skipped) ; (skipped) ; 0x1010: mov r1,#7
        let code = [0xE28F_0000, 0xE28F_F004, 0, 0, 0xE3A0_1007];
        let (c, _) = run_arm(&code, 3);
        assert_eq!(c.r[0], 0x1008);
        assert_eq!(c.r[1], 7);
    }
}
