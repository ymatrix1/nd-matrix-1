//! Kind numbers and operand layout of the move range `0x300..0x400`, shared by the lowering
//! (`lower/moves`, which writes them) and the handlers (which read them).
//!
//! OWNER: W2-MOVE. Operand `a` always carries the instruction length in its top two bits
//! (`mem::LEN_SHIFT`), the rest depends on the kind:
//!
//! | kind | `a` (low bits first) | `b` |
//! |---|---|---|
//! | `XFER` family (26 of 30 numbers) | `ii` 4, `mi` 4, `ureg` 7 | offset, delta or absolute address |
//! | `LOAD_IMM`, +class | `ureg` 7 | the value |
//! | `IMM_STORE` | `ii` 4, `mi` 4 | the value |
//! | `MODIFY` | `src` 4, `dst` 4, `mi` 4 | 0 |
//! | `MODIFY_IMM`, `BITREV_IMM` | `src` 4, `dst` 4 | the data |
//! | `REG_MOVE` | `src` 7, `dst` 7 | 0 |
//! | `REG_SWAP` | `r` 4, `s` 4 | 0 |
//! | `DUAL` | DM: `i` 3, `m` 3, reg 4, write 1; PM: the same (`i` and `m` without the +8), 22 bits | 0 |
//!
//! `XFER` kinds are `MOVE_BASE + (write * 5 + mode) * 3 + class`: 30 numbers, one body, 26
//! handlers (type 4, the only post-modify with an immediate, has a 4-bit register field, so its
//! `S` and other-register numbers are never made).
use super::super::mem::LEN_SHIFT;
use crate::core::uop::types::UopKind;

/// Address mode: access at `Ii`, then `Ii = modified(Ii, Mm)` (circular when enabled).
pub(crate) const POST_M: u8 = 0;
/// Access at `Ii`, then `Ii = modified_by(Ii, imm)` (type 4 with the U bit).
pub(crate) const POST_IMM: u8 = 1;
/// Access at `Ii + Mm`, no update, no wrap.
pub(crate) const PRE_M: u8 = 2;
/// Access at `Ii + imm`, no update.
pub(crate) const PRE_IMM: u8 = 3;
/// Access at the absolute address `imm` (type 14a).
pub(crate) const DIRECT: u8 = 4;
/// Number of address modes.
pub(crate) const MODES: u8 = 5;

/// Register class: a data register of PEx (`R0..R15`, code `0x00..=0x0F`), its PEy twin is `S`.
pub(crate) const CLASS_R: u8 = 0;
/// A data register of PEy (`S0..S15`, code `0x50..=0x5F`), its twin is `R`.
pub(crate) const CLASS_S: u8 = 1;
/// Any other universal register below the system registers (`I`, `M`, `L`, `B`): no twin.
pub(crate) const CLASS_OTHER: u8 = 2;

/// First universal register code of the system registers, which the move uops leave to the
/// interpreter (their reads and writes depend on `pc`, `steps` and the interrupt state).
/// VERIFIED: the same bound as `cut.rs::SYS_UREG`, checked by `classes_split_the_register_file`.
pub(crate) const SYS_UREG: u8 = 0x60;

/// The register class of `ureg`, `None` for a system register.
pub(crate) const fn class_of(ureg: u8) -> Option<u8> {
    match ureg {
        0x00..=0x0F => Some(CLASS_R),
        0x50..=0x5F => Some(CLASS_S),
        0x10..SYS_UREG => Some(CLASS_OTHER),
        _ => None,
    }
}

/// The kind of the transfer handler for a load or store (`write`), `mode` and register `class`.
pub(crate) const fn xfer_kind(write: bool, mode: u8, class: u8) -> UopKind {
    UopKind(UopKind::MOVE_BASE + (write as u16 * MODES as u16 + mode as u16) * 3 + class as u16)
}

/// `Un = <imm>`, plus the register class (3 kinds).
pub(crate) const LOAD_IMM: u16 = UopKind::MOVE_BASE + 0x20;
/// `DM/PM(Ii, Mm) = <imm>`, post-modify.
pub(crate) const IMM_STORE: UopKind = UopKind(UopKind::MOVE_BASE + 0x24);
/// `Id = MODIFY(Is, Mm)`.
pub(crate) const MODIFY: UopKind = UopKind(UopKind::MOVE_BASE + 0x25);
/// `Id = MODIFY(Is, <imm>)`.
pub(crate) const MODIFY_IMM: UopKind = UopKind(UopKind::MOVE_BASE + 0x26);
/// `Id = BITREV(Is, <imm>)`.
pub(crate) const BITREV_IMM: UopKind = UopKind(UopKind::MOVE_BASE + 0x27);
/// `Ureg = Ureg`.
pub(crate) const REG_MOVE: UopKind = UopKind(UopKind::MOVE_BASE + 0x28);
/// `Rr <-> Ss`.
pub(crate) const REG_SWAP: UopKind = UopKind(UopKind::MOVE_BASE + 0x29);
/// Dual data move without compute.
pub(crate) const DUAL: UopKind = UopKind(UopKind::MOVE_BASE + 0x2A);

/// The kind of `LOAD_IMM` for register class `class`.
pub(crate) const fn load_imm_kind(class: u8) -> UopKind {
    UopKind(LOAD_IMM + class as u16)
}

/// `a` of a transfer or an immediate store.
pub(crate) const fn xfer_a(ii: u8, mi: u8, ureg: u8, len: u32) -> u32 {
    ii as u32 | (mi as u32) << 4 | (ureg as u32) << 8 | len << LEN_SHIFT
}

/// `a` of the two-register kinds: low register, high register at bit `shift`.
pub(crate) const fn pair_a(lo: u8, hi: u8, shift: u32, len: u32) -> u32 {
    lo as u32 | (hi as u32) << shift | len << LEN_SHIFT
}

/// Index register field of `a`.
#[inline(always)]
pub(crate) const fn a_ii(a: u32) -> usize {
    (a & 15) as usize
}

/// Modify register field of `a`.
#[inline(always)]
pub(crate) const fn a_mi(a: u32) -> usize {
    (a >> 4 & 15) as usize
}

/// Universal register field of `a`.
#[inline(always)]
pub(crate) const fn a_ureg(a: u32) -> u8 {
    (a >> 8 & 0x7F) as u8
}

/// One side (DM or PM) of a dual move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Side {
    /// Index register, `0..16` (DM side `0..8`, PM side `8..16`).
    pub i: u8,
    /// Modify register, `0..16` (DM side `0..8`, PM side `8..16`).
    pub m: u8,
    /// Data register `R0..R15`.
    pub reg: u8,
    pub write: bool,
}

/// The 11 bits of one side of a dual move.
const fn side_bits(s: Side) -> u32 {
    (s.i & 7) as u32 | ((s.m & 7) as u32) << 3 | (s.reg as u32) << 6 | (s.write as u32) << 10
}

/// One side from its 11 bits and the DAG bank offset (0 for DM, 8 for PM).
#[inline(always)]
const fn side_of(bits: u32, bank: u8) -> Side {
    Side {
        i: (bits & 7) as u8 + bank,
        m: (bits >> 3 & 7) as u8 + bank,
        reg: (bits >> 6 & 15) as u8,
        write: bits >> 10 & 1 != 0,
    }
}

/// `a` of a dual move.
pub(crate) const fn dual_a(dm: Side, pm: Side, len: u32) -> u32 {
    side_bits(dm) | side_bits(pm) << 11 | len << LEN_SHIFT
}

/// The sides of a dual move from its `a`.
#[inline(always)]
pub(crate) const fn dual_sides(a: u32) -> (Side, Side) {
    (side_of(a, 0), side_of(a >> 11, 8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::KindFamily;

    #[test]
    fn classes_split_the_register_file() {
        for code in 0..0x80u8 {
            let want = match code >> 4 {
                0 => Some(CLASS_R),
                5 => Some(CLASS_S),
                1..=4 => Some(CLASS_OTHER),
                _ => None,
            };
            assert_eq!(class_of(code), want, "{code:#x}");
        }
        assert_eq!(SYS_UREG, 0x60);
    }

    #[test]
    fn kinds_are_distinct_and_inside_the_move_range() {
        let mut seen = std::collections::BTreeSet::new();
        for write in [false, true] {
            for mode in 0..MODES {
                for class in 0..3 {
                    assert!(seen.insert(xfer_kind(write, mode, class).0));
                }
            }
        }
        assert_eq!(seen.len(), 30);
        for class in 0..3 {
            assert!(seen.insert(load_imm_kind(class).0));
        }
        for k in [
            IMM_STORE, MODIFY, MODIFY_IMM, BITREV_IMM, REG_MOVE, REG_SWAP, DUAL,
        ] {
            assert!(seen.insert(k.0), "{k:?}");
        }
        for k in &seen {
            assert_eq!(UopKind(*k).family(), Some(KindFamily::Move));
        }
        assert_eq!(xfer_kind(false, POST_M, CLASS_R).0, UopKind::MOVE_BASE);
        assert_eq!(seen.last().copied(), Some(UopKind::MOVE_BASE + 0x2A));
    }

    #[test]
    fn operands_round_trip() {
        let a = xfer_a(15, 9, 0x5F, 3);
        assert_eq!(
            (a_ii(a), a_mi(a), a_ureg(a), a >> LEN_SHIFT),
            (15, 9, 0x5F, 3)
        );
        let dm = Side {
            i: 7,
            m: 3,
            reg: 15,
            write: true,
        };
        let pm = Side {
            i: 15,
            m: 8,
            reg: 0,
            write: false,
        };
        let a = dual_a(dm, pm, 2);
        assert_eq!(dual_sides(a), (dm, pm));
        assert_eq!(a >> LEN_SHIFT, 2);
        assert_eq!(pair_a(0x7F, 0x7F, 8, 3) & 0x3FFF_FFFF, 0x7F7F);
    }
}
