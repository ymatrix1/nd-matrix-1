//! OWNER: W1-XMEM. Executor arms of the memory uops (kinds `112..=128`, family `k::Family::Mem`).
//!
//! Spec (design note 3.1 and 4.3, WP W1-XMEM): arms generic over [`SbBus`] with the semantics of
//! the interpreter's `ldst_exec`, `arm_ldst_half` and `arm_ldm`: a word load rotates by
//! `8 * (a & 3)`, a word store goes to `a & !3`, halfword accesses to `a & !1`, `LDRSB`/`LDRSH`
//! sign-extend, `LDRD`/`STRD` move the pair `rd, rd + 1`, `LDM`/`STM` compute their address range
//! once (`block_span`) and then access the registers in ascending order. A load writes the base
//! back before it writes `rd`, so `rn == rd` leaves the loaded value; a store reads `rd` before
//! the write-back; an `STM` writes the base back after the store of its lowest register (a base
//! that is not the lowest register of the list is stored updated), an `LDM` skips the write-back
//! when the base is in the list.
//!
//! # Uop field layout (the table of design note 3.1, restated where it was ambiguous)
//!
//! | Kinds | `rd` | `rn` | `rm` | `rs` | `sh` | `imm` |
//! |---|---|---|---|---|---|---|
//! | `LDR_IMM LDRB_IMM STR_IMM STRB_IMM` | Rd | Rn | | `F_U \| F_P \| F_WB` | | 12-bit offset |
//! | `LDR_REG LDRB_REG STR_REG STRB_REG` | Rd | Rn | Rm | flags | `type << 5 \| amount` | |
//! | `LDRH LDRSH LDRSB STRH LDRD STRD` | Rd | Rn | Rm (`F_REG`) | flags, `F_REG` | | 8-bit offset |
//! | `LDM STM` | | Rn | | `F_U \| F_P \| F_WB` | popcount of the list | register list |
//! | `LDR_ABS` | Rd | | | | | absolute address |
//!
//! `F_WB` of a word/byte/half/double uop is `!P || W` of the instruction; of `LDM`/`STM` it is the
//! `W` bit alone (a post-indexed block transfer without `W` does not write back). A register
//! offset shifts by `alu::shift(type, amount, Rm, _, imm = true)`, so `LSR/ASR #0` mean 32; `ROR
//! #0` (RRX) needs the carry flag, which this signature does not carry, so the lowerer keeps it
//! `GENERIC` and the arm refuses it. No arm touches `r15` (I2): the lowerers keep `rd`, `rn`, `rm`
//! `== 15`, a loaded `r15`, `r15` in a list, an `S` bit and `ldrd`/`strd` pairs that reach `r15`
//! out of this family, and `debug_assert!` checks it.
//!
//! `bus.set_pc(u.pc)` is called once before the first access of a uop: every access of one
//! instruction carries the same pc, so this equals "before every access" (I5). The arm does not
//! look at `bus.stale()`: the executor loop does, after a store (I6).
//!
//! The tests compare every arm with the interpreter's handler on a logging bus (registers and
//! the ordered access log with pcs): `exec_mem/diff.rs` is the harness and the field table,
//! `fuzz.rs` the random suites and the mutation check, `cases.rs` hand-computed cases and
//! `contract.rs` the entry-point contract. Mutant sites ([`MUTANTS`]) let the mutation layer
//! check that the suites notice each rule.

use super::bus::SbBus;
use super::uop::{k, Uop};
use crate::cpu::alu::shift;
use crate::cpu::Cpu;

/// VERIFIED: `Uop::rs` bit 0, add the offset (else subtract) = design note 3.1 table, checked by
/// `flag_bits_follow_the_design_table`.
pub const F_U: u8 = 1;
/// VERIFIED: `Uop::rs` bit 1, pre-indexed (access at base +/- offset, else at the base); same check.
pub const F_P: u8 = 2;
/// VERIFIED: `Uop::rs` bit 2, write the updated base back (`!P || W`; `W` for `LDM`/`STM`); same check.
pub const F_WB: u8 = 4;
/// VERIFIED: `Uop::rs` bit 3 of the half/double kinds, the offset is in `Rm` (else `imm`); same check.
pub const F_REG: u8 = 8;

/// A mutation site (layer L5): always false outside tests, so the wrong branch is dead code there.
#[cfg(test)]
#[inline(always)]
fn mutant(id: u32) -> bool {
    super::mutants::is_active(id)
}

#[cfg(not(test))]
#[inline(always)]
const fn mutant(_id: u32) -> bool {
    false
}

/// Execute one load/store uop on the registers `r` through `bus`.
pub fn exec_mem<B: SbBus>(r: &mut [u32; 16], bus: &mut B, u: &Uop) {
    match u.kind {
        k::LDR_IMM => single::<B, true, false, false>(r, bus, u),
        k::LDRB_IMM => single::<B, true, true, false>(r, bus, u),
        k::STR_IMM => single::<B, false, false, false>(r, bus, u),
        k::STRB_IMM => single::<B, false, true, false>(r, bus, u),
        k::LDR_REG => single::<B, true, false, true>(r, bus, u),
        k::LDRB_REG => single::<B, true, true, true>(r, bus, u),
        k::STR_REG => single::<B, false, false, true>(r, bus, u),
        k::STRB_REG => single::<B, false, true, true>(r, bus, u),
        k::LDRH => half::<B, { k::LDRH }>(r, bus, u),
        k::LDRSH => half::<B, { k::LDRSH }>(r, bus, u),
        k::LDRSB => half::<B, { k::LDRSB }>(r, bus, u),
        k::STRH => half::<B, { k::STRH }>(r, bus, u),
        k::LDRD => half::<B, { k::LDRD }>(r, bus, u),
        k::STRD => half::<B, { k::STRD }>(r, bus, u),
        k::LDM => block::<B, true>(r, bus, u),
        k::STM => block::<B, false>(r, bus, u),
        k::LDR_ABS => {
            debug_assert!(u.rd != 15, "I2: a load into r15 is a block ender");
            publish_pc(bus, u);
            r[u.rd as usize] = if mutant(365) {
                bus.read32(u.imm & !3)
            } else {
                Cpu::load32(bus, u.imm)
            };
        }
        _ => unreachable!("uop kind {} is not a memory uop (OWNER W1-XMEM)", u.kind),
    }
}

/// Tell the bus which guest instruction accesses memory (I5).
#[inline(always)]
fn publish_pc<B: SbBus>(bus: &mut B, u: &Uop) {
    if !mutant(362) {
        bus.set_pc(u.pc);
    }
}

/// `base +/- off` as the U flag says.
#[inline(always)]
fn step_base(base: u32, off: u32, flags: u8) -> u32 {
    if (flags & F_U != 0) != mutant(353) {
        base.wrapping_add(off)
    } else {
        base.wrapping_sub(off)
    }
}

/// The address of a single transfer: the updated base when pre-indexed, else the base.
#[inline(always)]
fn access_addr(base: u32, moved: u32, flags: u8) -> u32 {
    if flags & F_P != 0 || mutant(354) {
        moved
    } else {
        base
    }
}

/// A word load: aligned read, rotated right by `8 * (a & 3)` (the pre-ARMv6 rule of `Cpu::load32`).
#[inline(always)]
fn load_word<B: SbBus>(bus: &mut B, addr: u32) -> u32 {
    if mutant(350) {
        bus.read32(addr & !3)
    } else {
        Cpu::load32(bus, addr)
    }
}

/// Finish a load: the base is written back first (skipped when it is `rd`), then `rd` is written,
/// so a load with `rn == rd` leaves the loaded value.
#[inline(always)]
fn finish_load(r: &mut [u32; 16], u: &Uop, moved: u32, v: u32) {
    let (rn, rd) = (u.rn as usize, u.rd as usize);
    let wb = u.rs & F_WB != 0;
    if mutant(352) {
        r[rd] = v;
        if wb {
            r[rn] = moved;
        }
        return;
    }
    if wb && rn != rd {
        r[rn] = moved;
    }
    r[rd] = v;
}

/// The offset of a register-offset word/byte transfer. RRX needs the carry flag the arm does not
/// have: the lowerer must keep it `GENERIC`.
#[inline(always)]
fn reg_offset(r: &[u32; 16], u: &Uop) -> u32 {
    let (ty, amount) = (u32::from(u.sh >> 5 & 3), u32::from(u.sh & 31));
    assert!(
        !(ty == 3 && amount == 0),
        "register offset ROR #0 (RRX) needs the carry flag: the lowerer keeps it GENERIC"
    );
    let ty = if mutant(364) { 0 } else { ty };
    shift(ty, amount, r[u.rm as usize], false, true).0
}

/// `ldr`/`ldrb`/`str`/`strb` with an immediate or a shifted register offset.
fn single<B: SbBus, const LOAD: bool, const BYTE: bool, const REG_OFF: bool>(
    r: &mut [u32; 16],
    bus: &mut B,
    u: &Uop,
) {
    let (rn, rd) = (u.rn as usize, u.rd as usize);
    debug_assert!(
        rn != 15 && rd != 15,
        "I2: no uop but GENERIC*/EXIT* names r15"
    );
    let base = r[rn];
    let off = if REG_OFF { reg_offset(r, u) } else { u.imm };
    let moved = step_base(base, off, u.rs);
    let addr = access_addr(base, moved, u.rs);
    publish_pc(bus, u);
    if LOAD {
        let v = if BYTE {
            u32::from(bus.read8(addr))
        } else {
            load_word(bus, addr)
        };
        finish_load(r, u, moved, v);
    } else {
        let v = r[rd];
        if BYTE {
            bus.write8(addr, v as u8);
        } else {
            bus.write32(if mutant(351) { addr } else { addr & !3 }, v);
        }
        if u.rs & F_WB != 0 && !mutant(366) {
            r[rn] = moved;
        }
    }
}

/// The halfword address rule: bit 0 is ignored.
#[inline(always)]
fn half_addr(addr: u32) -> u32 {
    if mutant(355) {
        addr
    } else {
        addr & !1
    }
}

/// `ldrh`, `ldrsh`, `ldrsb`, `strh`, `ldrd`, `strd`; `KIND` is the uop kind.
fn half<B: SbBus, const KIND: u8>(r: &mut [u32; 16], bus: &mut B, u: &Uop) {
    let (rn, rd) = (u.rn as usize, u.rd as usize);
    debug_assert!(
        rn != 15 && rd != 15,
        "I2: no uop but GENERIC*/EXIT* names r15"
    );
    debug_assert!(
        rd < 14 || !matches!(KIND, k::LDRD | k::STRD),
        "I2: the pair of ldrd/strd must not reach r15"
    );
    let base = r[rn];
    let off = if u.rs & F_REG != 0 {
        r[u.rm as usize]
    } else {
        u.imm
    };
    let moved = step_base(base, off, u.rs);
    let addr = access_addr(base, moved, u.rs);
    let wb = u.rs & F_WB != 0;
    publish_pc(bus, u);
    match KIND {
        k::LDRH => {
            let v = u32::from(bus.read16(half_addr(addr)));
            finish_load(r, u, moved, v);
        }
        k::LDRSB => {
            let b = bus.read8(addr);
            let v = if mutant(356) {
                u32::from(b)
            } else {
                b as i8 as i32 as u32
            };
            finish_load(r, u, moved, v);
        }
        k::LDRSH => {
            let h = bus.read16(half_addr(addr));
            let v = if mutant(357) {
                u32::from(h)
            } else {
                h as i16 as i32 as u32
            };
            finish_load(r, u, moved, v);
        }
        k::STRH => {
            bus.write16(half_addr(addr), r[rd] as u16);
            if wb {
                r[rn] = moved;
            }
        }
        k::LDRD => {
            let a = addr & !3;
            let lo = bus.read32(a);
            let hi = bus.read32(a.wrapping_add(4));
            if mutant(358) {
                (r[rd], r[(rd + 1) & 15]) = (lo, hi);
                if wb {
                    r[rn] = moved;
                }
            } else {
                // The pair lands after the write-back, so it wins when `rn` is `rd` or `rd + 1`.
                if wb {
                    r[rn] = moved;
                }
                (r[rd], r[(rd + 1) & 15]) = (lo, hi);
            }
        }
        k::STRD => {
            let a = addr & !3;
            let (lo, hi) = (r[rd], r[(rd + 1) & 15]);
            if mutant(363) {
                bus.write32(a.wrapping_add(4), hi);
                bus.write32(a, lo);
            } else {
                bus.write32(a, lo);
                bus.write32(a.wrapping_add(4), hi);
            }
            if wb {
                r[rn] = moved;
            }
        }
        _ => unreachable!("kind {KIND} is not a half/double uop (OWNER W1-XMEM)"),
    }
}

/// The one address computation of a block transfer of `n` registers from `base`: the lowest
/// address accessed and the written-back base. `flags` carries U and P (IA, IB, DA, DB).
#[inline(always)]
fn block_span(base: u32, flags: u8, n: u32) -> (u32, u32) {
    let bytes = 4 * n;
    let up = flags & F_U != 0;
    let low = match (up, flags & F_P != 0) {
        (true, false) => base,
        (true, true) => base.wrapping_add(4),
        (false, false) => base.wrapping_sub(bytes).wrapping_add(4),
        (false, true) if mutant(361) => base.wrapping_sub(bytes).wrapping_sub(4),
        (false, true) => base.wrapping_sub(bytes),
    };
    let wb_val = if up {
        base.wrapping_add(bytes)
    } else {
        base.wrapping_sub(bytes)
    };
    (low, wb_val)
}

/// `ldm` / `stm` (S = 0, no `r15` in the list).
fn block<B: SbBus, const LOAD: bool>(r: &mut [u32; 16], bus: &mut B, u: &Uop) {
    let rn = u.rn as usize;
    let list = u.imm;
    debug_assert!(
        rn != 15 && list >> 15 == 0,
        "I2: no uop but GENERIC*/EXIT* names r15"
    );
    debug_assert_eq!(
        u32::from(u.sh),
        list.count_ones(),
        "sh is the popcount of the list"
    );
    let (low, wb_val) = block_span(r[rn], u.rs, u32::from(u.sh));
    let wb = u.rs & F_WB != 0;
    publish_pc(bus, u);
    let (mut addr, mut rest) = (low, list);
    if LOAD {
        while rest != 0 {
            let i = rest.trailing_zeros() as usize;
            rest &= rest - 1;
            r[i] = bus.read32(addr & !3);
            addr = addr.wrapping_add(4);
        }
        if wb && (list >> rn & 1 == 0 || mutant(360)) {
            r[rn] = wb_val;
        }
    } else {
        let first = list.trailing_zeros(); // 32 for an empty list: never a register index
        while rest != 0 {
            let i = rest.trailing_zeros();
            rest &= rest - 1;
            bus.write32(addr & !3, r[i as usize]);
            addr = addr.wrapping_add(4);
            if wb && (if mutant(359) { rest == 0 } else { i == first }) {
                r[rn] = wb_val;
            }
        }
    }
}

#[cfg(test)]
mod cases;
#[cfg(test)]
mod contract;
#[cfg(test)]
mod diff;
#[cfg(test)]
mod fuzz;
#[cfg(test)]
pub use fuzz::MUTANTS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_bits_follow_the_design_table() {
        assert_eq!((F_U, F_P, F_WB, F_REG), (1, 2, 4, 8));
    }
}
