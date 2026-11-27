//! ARM state decoder: condition check, branches, status registers, coprocessor 15, dispatch.

use super::{Bus, Cpu, Fault, CP15_ID};

/// ASSUMED: ARM926EJ-S cache type register as in the TRM example (`0x0D152152`: 16 KB I and D
/// caches); firmware only reads it for cache maintenance sizing, confirm with the real core.
const CP15_CACHE_TYPE: u32 = 0x0D15_2152;

impl Cpu {
    #[inline(always)]
    pub(crate) fn cond(&self, cond: u32) -> bool {
        match cond {
            0 => self.z,
            1 => !self.z,
            2 => self.c,
            3 => !self.c,
            4 => self.n,
            5 => !self.n,
            6 => self.v,
            7 => !self.v,
            8 => self.c && !self.z,
            9 => !self.c || self.z,
            10 => self.n == self.v,
            11 => self.n != self.v,
            12 => !self.z && self.n == self.v,
            13 => self.z || self.n != self.v,
            _ => true,
        }
    }

    fn undef(pc: u32, insn: u32) -> Result<(), Fault> {
        Err(Fault::Undefined { pc, insn })
    }

    pub(crate) fn exec_arm<B: Bus>(
        &mut self,
        bus: &mut B,
        pc: u32,
        insn: u32,
    ) -> Result<(), Fault> {
        let cond = insn >> 28;
        if cond == 0xF {
            return self.exec_arm_uncond(bus, pc, insn);
        }
        if !self.cond(cond) {
            return Ok(());
        }
        self.exec_arm_body(bus, pc, insn)
    }

    /// Everything after the condition check (the predecode cache tests the condition itself).
    pub(crate) fn exec_arm_body<B: Bus>(
        &mut self,
        bus: &mut B,
        pc: u32,
        insn: u32,
    ) -> Result<(), Fault> {
        match insn >> 25 & 7 {
            0 => self.arm_000(bus, pc, insn),
            1 => self.arm_001(pc, insn),
            2 | 3 => {
                if insn >> 25 & 1 != 0 && insn >> 4 & 1 != 0 {
                    return Self::undef(pc, insn);
                }
                self.arm_ldst(bus, insn);
                Ok(())
            }
            4 => {
                self.arm_ldm(bus, insn);
                Ok(())
            }
            5 => {
                let off = (((insn & 0x00FF_FFFF) << 8) as i32 >> 6) as u32;
                if insn >> 24 & 1 != 0 {
                    self.r[14] = pc.wrapping_add(4);
                }
                self.branch(self.r[15].wrapping_add(off), false);
                Ok(())
            }
            6 => Self::undef(pc, insn), // ldc / stc: no coprocessor model
            _ => {
                if insn >> 24 & 1 != 0 {
                    return Err(Fault::Swi {
                        pc,
                        imm: insn & 0x00FF_FFFF,
                    });
                }
                self.arm_coproc(pc, insn)
            }
        }
    }

    pub(crate) fn exec_arm_uncond<B: Bus>(
        &mut self,
        bus: &mut B,
        pc: u32,
        insn: u32,
    ) -> Result<(), Fault> {
        if insn >> 25 & 7 == 5 {
            // BLX <imm>
            let off = ((((insn & 0x00FF_FFFF) << 8) as i32 >> 6) as u32) | (insn >> 23 & 2);
            self.r[14] = pc.wrapping_add(4);
            self.thumb = true;
            self.branch(self.r[15].wrapping_add(off), false);
            return Ok(());
        }
        // PLD (cache hint): no effect.
        if insn & 0xFD70_F000 == 0xF550_F000 {
            let _ = bus;
            return Ok(());
        }
        Self::undef(pc, insn)
    }

    fn arm_000<B: Bus>(&mut self, bus: &mut B, pc: u32, insn: u32) -> Result<(), Fault> {
        let bit4 = insn >> 4 & 1 != 0;
        let bit7 = insn >> 7 & 1 != 0;
        if bit4 && bit7 {
            // multiply, swap, halfword transfers
            return if insn >> 5 & 3 == 0 {
                if insn >> 24 & 1 != 0 {
                    self.arm_swp(bus, insn);
                    Ok(())
                } else {
                    self.arm_mul(insn);
                    Ok(())
                }
            } else {
                self.arm_ldst_half(bus, pc, insn)
            };
        }
        let op = insn >> 21 & 0xF;
        let s = insn >> 20 & 1 != 0;
        if (8..=11).contains(&op) && !s {
            return self.arm_misc(bus, pc, insn);
        }
        self.arm_dp(pc, insn);
        Ok(())
    }

    fn arm_001(&mut self, pc: u32, insn: u32) -> Result<(), Fault> {
        let op = insn >> 21 & 0xF;
        let s = insn >> 20 & 1 != 0;
        if (8..=11).contains(&op) && !s {
            if op == 9 || op == 11 {
                // MSR immediate
                let v = super::alu::arm_imm(insn & 0xFF, insn >> 8 & 0xF);
                self.msr(insn, v);
                return Ok(());
            }
            return Self::undef(pc, insn);
        }
        self.arm_dp(pc, insn);
        Ok(())
    }

    /// Miscellaneous instructions in the data-processing hole (TST/TEQ/CMP/CMN without S).
    fn arm_misc<B: Bus>(&mut self, bus: &mut B, pc: u32, insn: u32) -> Result<(), Fault> {
        let _ = bus;
        let rm = (insn & 0xF) as usize;
        let rd = (insn >> 12 & 0xF) as usize;
        let sub = insn >> 4 & 0xF;
        match (insn >> 21 & 0xF, sub) {
            (8, 0) => self.r[rd] = self.cpsr(),
            (10, 0) => self.r[rd] = self.spsr,
            (9, 0) => self.msr(insn, self.r[rm]),
            (11, 0) => self.spsr = self.r[rm],
            (9, 1) => {
                let t = self.r[rm];
                self.branch(t, true);
            }
            (9, 3) => {
                let t = self.r[rm];
                self.r[14] = pc.wrapping_add(4);
                self.branch(t, true);
            }
            (11, 1) => self.r[rd] = self.r[rm].leading_zeros(),
            (9, 7) => return Self::undef(pc, insn), // bkpt
            (8..=11, 5) => self.arm_qaddsub(insn),
            (8..=11, 8..=15) => self.arm_dsp_mul(insn),
            _ => return Self::undef(pc, insn),
        }
        Ok(())
    }

    /// MSR: write the CPSR (or SPSR for `insn` bit 22) through the field mask.
    fn msr(&mut self, insn: u32, value: u32) {
        let mut mask = 0u32;
        if insn >> 19 & 1 != 0 {
            mask |= 0xFF00_0000;
        }
        if insn >> 16 & 1 != 0 {
            mask |= 0x0000_00DF;
        }
        if insn >> 22 & 1 != 0 {
            self.spsr = (self.spsr & !mask) | (value & mask);
            return;
        }
        let old_mode = self.mode;
        let mixed = (self.cpsr() & !mask) | (value & mask);
        let keep_t = self.thumb;
        self.set_cpsr(mixed);
        self.thumb = keep_t;
        if self.mode != old_mode {
            self.mode_changes += 1;
        }
    }

    fn arm_coproc(&mut self, pc: u32, insn: u32) -> Result<(), Fault> {
        let cp = insn >> 8 & 0xF;
        if cp != 15 || insn >> 4 & 1 == 0 {
            return Self::undef(pc, insn);
        }
        let rd = (insn >> 12 & 0xF) as usize;
        if insn >> 20 & 1 != 0 {
            // MRC p15: ID, control register; everything else reads back 0
            let v = match (insn >> 16 & 0xF, insn >> 5 & 7, insn & 0xF) {
                (0, 0, 0) => CP15_ID,
                (1, 0, 0) => self.cp15_ctrl,
                (0, 1, 0) => CP15_CACHE_TYPE,
                _ => 0,
            };
            if rd == 15 {
                self.n = v >> 31 != 0;
                self.z = v >> 30 & 1 != 0;
                self.c = v >> 29 & 1 != 0;
                self.v = v >> 28 & 1 != 0;
            } else {
                self.r[rd] = v;
            }
        } else if insn >> 16 & 0xF == 1 && insn & 0xF == 0 {
            self.cp15_ctrl = self.r[rd];
        } else if insn >> 16 & 0xF == 7 && insn & 0xF == 0 && insn >> 5 & 7 == 4 {
            return Err(Fault::Wfi);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::testkit::run_arm;

    #[test]
    fn branch_and_link_and_conditions() {
        // 1000: mov r0,#1 ; 1004: bl 0x1010 ; 1008: mov r2,#9 ; 100c: b . ;
        // 1010: mov r1,#3 ; 1014: mov pc,lr
        let code = [
            0xE3A0_0001,
            0xEB00_0001,
            0xE3A0_2009,
            0xEAFF_FFFE,
            0xE3A0_1003,
            0xE1A0_F00E,
        ];
        let (c, _) = run_arm(&code, 5);
        assert_eq!((c.r[1], c.r[2], c.r[14]), (3, 9, 0x1008));
    }

    #[test]
    fn conditional_skip_uses_flags() {
        // cmp r0,r0 ; movne r1,#1 ; moveq r2,#2
        let (c, _) = run_arm(&[0xE150_0000, 0x13A0_1001, 0x03A0_2002], 3);
        assert_eq!((c.r[1], c.r[2]), (0, 2));
    }

    #[test]
    fn bx_enters_thumb_and_blx_imm_sets_link() {
        // mov r0,#0x1011 (via add) ; bx r0
        let code = [0xE3A0_0011, 0xE380_0B04, 0xE12F_FF10];
        let (c, _) = run_arm(&code, 3);
        assert!(c.thumb);
        assert_eq!(c.r[15], 0x1010);
    }

    #[test]
    fn wfi_write_to_cp15_c7_reports_wfi_after_advancing_pc() {
        let mut bus = crate::cpu::bus::FlatBus::default();
        bus.0.write32(0x1000, 0xEE07_0F90); // mcr p15,0,r0,c7,c0,4
        let mut c = crate::cpu::Cpu::new();
        c.r[15] = 0x1000;
        assert_eq!(c.step(&mut bus), Err(crate::cpu::Fault::Wfi));
        assert_eq!(c.r[15], 0x1004);
    }

    #[test]
    fn clz_mrs_msr_and_cp15_id() {
        // mov r0,#0x10 ; clz r1,r0 ; mrc p15,0,r2,c0,c0,0 ; mrs r3,cpsr
        let code = [0xE3A0_0010, 0xE16F_1F10, 0xEE10_2F10, 0xE10F_3000];
        let (c, _) = run_arm(&code, 4);
        assert_eq!(c.r[1], 27);
        assert_eq!(c.r[2], 0x4106_9265);
        assert_eq!(c.r[3] & 0x1F, 0x13);
    }
}
