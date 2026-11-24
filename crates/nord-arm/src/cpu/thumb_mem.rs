//! Thumb load/store, push/pop, stack-pointer arithmetic and block transfers.

use super::{Bus, Cpu, Fault};

impl Cpu {
    pub(crate) fn thumb_ldst<B: Bus>(
        &mut self,
        bus: &mut B,
        _pc: u32,
        insn: u32,
    ) -> Result<(), Fault> {
        let rd = (insn & 7) as usize;
        let rn = (insn >> 3 & 7) as usize;
        match insn >> 11 {
            10 | 11 => {
                let addr = self.r[rn].wrapping_add(self.r[(insn >> 6 & 7) as usize]);
                match insn >> 9 & 7 {
                    0 => bus.write32(addr & !3, self.r[rd]),
                    1 => bus.write16(addr & !1, self.r[rd] as u16),
                    2 => bus.write8(addr, self.r[rd] as u8),
                    3 => self.r[rd] = bus.read8(addr) as i8 as i32 as u32,
                    4 => self.r[rd] = Self::load32(bus, addr),
                    5 => self.r[rd] = bus.read16(addr & !1) as u32,
                    6 => self.r[rd] = bus.read8(addr) as u32,
                    _ => self.r[rd] = bus.read16(addr & !1) as i16 as i32 as u32,
                }
            }
            12..=15 => {
                let imm = insn >> 6 & 0x1F;
                let byte = insn >> 12 & 1 != 0;
                let load = insn >> 11 & 1 != 0;
                let addr = self.r[rn].wrapping_add(if byte { imm } else { imm << 2 });
                match (byte, load) {
                    (false, false) => bus.write32(addr & !3, self.r[rd]),
                    (false, true) => self.r[rd] = Self::load32(bus, addr),
                    (true, false) => bus.write8(addr, self.r[rd] as u8),
                    (true, true) => self.r[rd] = bus.read8(addr) as u32,
                }
            }
            16 | 17 => {
                let addr = self.r[rn].wrapping_add((insn >> 6 & 0x1F) << 1);
                if insn >> 11 & 1 != 0 {
                    self.r[rd] = bus.read16(addr & !1) as u32;
                } else {
                    bus.write16(addr & !1, self.r[rd] as u16);
                }
            }
            _ => {
                let rd = (insn >> 8 & 7) as usize;
                let addr = self.r[13].wrapping_add((insn & 0xFF) << 2);
                if insn >> 11 & 1 != 0 {
                    self.r[rd] = Self::load32(bus, addr);
                } else {
                    bus.write32(addr & !3, self.r[rd]);
                }
            }
        }
        Ok(())
    }

    /// `add sp, #imm`, `push`, `pop`, `bkpt`.
    #[allow(clippy::needless_range_loop)] // register-list bit i selects register i
    pub(crate) fn thumb_misc<B: Bus>(
        &mut self,
        bus: &mut B,
        pc: u32,
        insn: u32,
    ) -> Result<(), Fault> {
        match insn >> 8 & 0xF {
            0 => {
                let off = (insn & 0x7F) << 2;
                self.r[13] = if insn >> 7 & 1 != 0 {
                    self.r[13].wrapping_sub(off)
                } else {
                    self.r[13].wrapping_add(off)
                };
            }
            4 | 5 => {
                // push {rlist[, lr]}
                let list = insn & 0xFF | if insn >> 8 & 1 != 0 { 1 << 14 } else { 0 };
                let mut addr = self.r[13].wrapping_sub(4 * list.count_ones());
                self.r[13] = addr;
                for i in 0..15usize {
                    if list >> i & 1 != 0 {
                        bus.write32(addr & !3, self.r[i]);
                        addr = addr.wrapping_add(4);
                    }
                }
            }
            12 | 13 => {
                // pop {rlist[, pc]}
                let mut addr = self.r[13];
                for i in 0..8usize {
                    if insn >> i & 1 != 0 {
                        self.r[i] = bus.read32(addr & !3);
                        addr = addr.wrapping_add(4);
                    }
                }
                if insn >> 8 & 1 != 0 {
                    let v = bus.read32(addr & !3);
                    addr = addr.wrapping_add(4);
                    self.r[13] = addr;
                    self.branch(v, true);
                } else {
                    self.r[13] = addr;
                }
            }
            _ => return Err(Fault::Undefined { pc, insn }),
        }
        Ok(())
    }

    /// `ldmia rn!, {rlist}` and `stmia rn!, {rlist}`.
    #[allow(clippy::needless_range_loop)] // register-list bit i selects register i
    pub(crate) fn thumb_ldm_stm<B: Bus>(&mut self, bus: &mut B, insn: u32) {
        let rn = (insn >> 8 & 7) as usize;
        let list = insn & 0xFF;
        let mut addr = self.r[rn];
        let wb = addr.wrapping_add(4 * list.count_ones());
        if insn >> 11 & 1 != 0 {
            for i in 0..8usize {
                if list >> i & 1 != 0 {
                    self.r[i] = bus.read32(addr & !3);
                    addr = addr.wrapping_add(4);
                }
            }
            if list >> rn & 1 == 0 {
                self.r[rn] = wb;
            }
        } else {
            for i in 0..8usize {
                if list >> i & 1 != 0 {
                    bus.write32(addr & !3, self.r[i]);
                    addr = addr.wrapping_add(4);
                }
            }
            self.r[rn] = wb;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::bus::Bus;
    use crate::cpu::testkit::run_thumb;

    #[test]
    fn push_pop_with_lr_and_pc_return() {
        // movs r0,#7 ; movs r3,#0x30 ; lsls r3,r3,#8 ; mov sp,r3 (0x469D) ;
        // push {r0,lr} (0xB501) ; movs r0,#0 ; pop {r1,pc} (0xBD02)
        let code = [0x2007, 0x2330, 0x021B, 0x469D, 0xB501, 0x2000, 0xBD02];
        let (c, mut b) = run_thumb(&code, 7);
        assert_eq!(c.r[1], 7);
        assert_eq!(c.r[13], 0x3000);
        assert_eq!(b.read32(0x2FF8), 7);
        // pop pc loaded lr (0): interwork bit 0 clear leaves Thumb state
        assert!(!c.thumb);
    }

    #[test]
    fn register_offset_loads_and_stores() {
        // movs r0,#0x20 ; lsls r0,r0,#8 (0x0200) ; movs r1,#4 ; movs r2,#0x5A ;
        // str r2,[r0,r1] (0x5042) ; ldr r3,[r0,r1] (0x5843) ; ldrb r4,[r0,r1] (0x5C44)
        let code = [0x2020, 0x0200, 0x2104, 0x225A, 0x5042, 0x5843, 0x5C44];
        let (c, _) = run_thumb(&code, 7);
        assert_eq!((c.r[3], c.r[4]), (0x5A, 0x5A));
    }

    #[test]
    fn ldmia_stmia_and_sp_relative() {
        // movs r0,#0x30 ; lsls r0,r0,#8 ; movs r1,#1 ; movs r2,#2 ; stmia r0!,{r1,r2} (0xC006) ;
        // subs r0,#8 ; ldmia r0!,{r3,r4} (0xC818)
        let code = [0x2030, 0x0200, 0x2101, 0x2202, 0xC006, 0x3808, 0xC818];
        let (c, _) = run_thumb(&code, 7);
        assert_eq!((c.r[3], c.r[4], c.r[0]), (1, 2, 0x3008));
    }
}
