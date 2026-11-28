//! OWNER: W1-MEM (test support). Reference semantics of the memory uops this lowerer emits,
//! written from the field table of the design note (3.1) and `lower_mem.rs`, not from the ARM
//! word. `semantics.rs` runs it against `Cpu::step`, so a wrongly packed field shows up as a
//! difference in registers or in the ordered bus access log.
//!
//! It exists because the real executor (`exec_mem`, W1-XMEM) is a different work package: the
//! lowerer's tests must not depend on it. The two are meant to agree; W3-VERIF's L0 proves it.

use super::{FLAG_P, FLAG_REG, FLAG_U, FLAG_WB};
use crate::cpu::alu::shift;
use crate::machine::sb::bus::SbBus;
use crate::machine::sb::uop::{k, Uop};

/// Run one uop of the families `lower_mem` emits (`MOV_IMM` of a folded literal included) on
/// registers `r` with carry `carry` (the shifted register offset needs it) through `bus`.
pub(super) fn run<B: SbBus>(r: &mut [u32; 16], carry: bool, bus: &mut B, u: &Uop) {
    let (rd, rn) = (u.rd as usize, u.rn as usize);
    match u.kind {
        k::MOV_IMM => r[rd] = u.imm,
        k::LDR_ABS => {
            bus.set_pc(u.pc);
            r[rd] = load_word(bus, u.imm);
        }
        k::LDM | k::STM => block(r, bus, u),
        k::LDR_IMM..=k::STRB_REG => {
            let (store, byte, reg) = (u.kind & 2 != 0, u.kind & 1 != 0, u.kind >= k::LDR_REG);
            let off = if reg {
                let (ty, amount) = ((u.sh >> 5) as u32, (u.sh & 31) as u32);
                shift(ty, amount, r[u.rm as usize], carry, true).0
            } else {
                u.imm
            };
            let (addr, moved) = address(r[rn], off, u);
            bus.set_pc(u.pc);
            if store {
                let v = r[rd];
                if byte {
                    bus.write8(addr, v as u8);
                } else {
                    bus.write32(addr & !3, v);
                }
                if u.rs & FLAG_WB != 0 {
                    r[rn] = moved;
                }
            } else {
                let v = if byte {
                    bus.read8(addr) as u32
                } else {
                    load_word(bus, addr)
                };
                finish_load(r, u, moved, v);
            }
        }
        k::LDRH..=k::STRD => {
            let off = if u.rs & FLAG_REG != 0 {
                r[u.rm as usize]
            } else {
                u.imm
            };
            let (addr, moved) = address(r[rn], off, u);
            bus.set_pc(u.pc);
            match u.kind {
                k::LDRH => {
                    let v = bus.read16(addr & !1) as u32;
                    finish_load(r, u, moved, v);
                }
                k::LDRSH => {
                    let v = bus.read16(addr & !1) as i16 as i32 as u32;
                    finish_load(r, u, moved, v);
                }
                k::LDRSB => {
                    let v = bus.read8(addr) as i8 as i32 as u32;
                    finish_load(r, u, moved, v);
                }
                k::STRH => {
                    bus.write16(addr & !1, r[rd] as u16);
                    write_back(r, u, moved);
                }
                k::LDRD => {
                    let lo = bus.read32(addr & !3);
                    let hi = bus.read32((addr & !3).wrapping_add(4));
                    write_back(r, u, moved);
                    (r[rd], r[rd + 1]) = (lo, hi);
                }
                _ => {
                    bus.write32(addr & !3, r[rd]);
                    bus.write32((addr & !3).wrapping_add(4), r[rd + 1]);
                    write_back(r, u, moved);
                }
            }
        }
        other => unreachable!("kind {other} is not a memory uop of this lowerer"),
    }
}

/// `(transfer address, moved base)` of base value `base` and offset `off`.
fn address(base: u32, off: u32, u: &Uop) -> (u32, u32) {
    let moved = if u.rs & FLAG_U != 0 {
        base.wrapping_add(off)
    } else {
        base.wrapping_sub(off)
    };
    (if u.rs & FLAG_P != 0 { moved } else { base }, moved)
}

fn load_word<B: SbBus>(bus: &mut B, addr: u32) -> u32 {
    bus.read32(addr & !3).rotate_right(8 * (addr & 3))
}

fn write_back(r: &mut [u32; 16], u: &Uop, moved: u32) {
    if u.rs & FLAG_WB != 0 {
        r[u.rn as usize] = moved;
    }
}

/// A load writes the base back first and the loaded value wins when `rn == rd`.
fn finish_load(r: &mut [u32; 16], u: &Uop, moved: u32, v: u32) {
    if u.rn != u.rd {
        write_back(r, u, moved);
    }
    r[u.rd as usize] = v;
}

/// `ldm`/`stm`: the lowest address of the block, registers in ascending order, `sh` = popcount.
fn block<B: SbBus>(r: &mut [u32; 16], bus: &mut B, u: &Uop) {
    let (list, n, rn) = (u.imm, u.sh as u32, u.rn as usize);
    assert_eq!(n, list.count_ones(), "sh must be the popcount of the list");
    assert_eq!(list & 0x8000, 0, "r15 never reaches a block uop");
    let base = r[rn];
    let low = match (u.rs & FLAG_U != 0, u.rs & FLAG_P != 0) {
        (true, false) => base,
        (true, true) => base.wrapping_add(4),
        (false, false) => base.wrapping_sub(4 * n).wrapping_add(4),
        (false, true) => base.wrapping_sub(4 * n),
    };
    let wb_val = if u.rs & FLAG_U != 0 {
        base.wrapping_add(4 * n)
    } else {
        base.wrapping_sub(4 * n)
    };
    let (wb, mut addr) = (u.rs & FLAG_WB != 0, low);
    bus.set_pc(u.pc);
    if u.kind == k::LDM {
        for i in (0..15).filter(|i| list >> i & 1 != 0) {
            r[i] = bus.read32(addr & !3);
            addr = addr.wrapping_add(4);
        }
        if wb && list >> rn & 1 == 0 {
            r[rn] = wb_val;
        }
    } else {
        let first = list.trailing_zeros() as usize;
        for i in (0..15).filter(|i| list >> i & 1 != 0) {
            bus.write32(addr & !3, r[i]);
            addr = addr.wrapping_add(4);
            if wb && i == first {
                r[rn] = wb_val;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testkit::LogBus;

    fn mem_uop(kind: u8, rd: u8, rn: u8, rs: u8, imm: u32) -> Uop {
        let mut u = Uop::new(kind, 0x1000);
        (u.rd, u.rn, u.rs, u.imm) = (rd, rn, rs, imm);
        u
    }

    #[test]
    fn the_model_does_what_the_field_table_says() {
        let mut bus = LogBus::new(1);
        bus.poke32(0x2004, 0x4433_2211);
        let mut r = [0u32; 16];
        r[1] = 0x2000;
        // ldr r0,[r1,#5]! : aligned down, rotated by 8 * 1, base written back
        let ldr = mem_uop(k::LDR_IMM, 0, 1, FLAG_U | FLAG_P | FLAG_WB, 5);
        run(&mut r, false, &mut bus, &ldr);
        assert_eq!((r[0], r[1]), (0x1144_3322, 0x2005));
        assert_eq!(
            bus.log,
            vec![(2, 0x2004, 0, 0x1000)],
            "the access is word aligned"
        );
        // post-index store of r0 at the old base, base moves down
        let str_ = mem_uop(k::STR_IMM, 0, 1, FLAG_WB, 8);
        run(&mut r, false, &mut bus, &str_);
        assert_eq!((r[1], bus.peek32(0x2004)), (0x2005 - 8, 0x1144_3322));
        // ldm ia r2!,{r2,r3}: the loaded base wins and nothing is written back
        let mut ldm = mem_uop(k::LDM, 0, 2, FLAG_U | FLAG_WB, 0b1100);
        (ldm.sh, r[2]) = (2, 0x2004);
        run(&mut r, false, &mut bus, &ldm);
        assert_eq!(r[2], 0x1144_3322);
        // r0 of a folded literal is just the constant
        run(
            &mut r,
            false,
            &mut bus,
            &mem_uop(k::MOV_IMM, 4, 0, 0, 0xABCD),
        );
        assert_eq!(r[4], 0xABCD);
    }

    #[test]
    #[should_panic(expected = "popcount")]
    fn a_wrong_popcount_is_caught_by_the_model() {
        let mut ldm = mem_uop(k::LDM, 0, 2, FLAG_U, 0b1100);
        ldm.sh = 3;
        run(&mut [0; 16], false, &mut LogBus::new(1), &ldm);
    }
}
