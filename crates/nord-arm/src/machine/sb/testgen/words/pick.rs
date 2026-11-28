//! OWNER: W1-VGEN. The field pickers of `words.rs`: registers, shift amounts, offsets, register
//! lists and the transfer encodings every family draws from, each with the distribution the
//! module docs of `words.rs` promise.

use super::{random_of, Family};
use crate::machine::sb::testkit::Rng;

/// A register number: `r0..r14`, `r15` one time in sixteen.
pub(super) fn reg(rng: &mut Rng) -> u32 {
    if rng.chance(16) {
        15
    } else {
        rng.below(15)
    }
}

/// A register number that is never `pc`.
pub(super) fn reg_np(rng: &mut Rng) -> u32 {
    rng.below(15)
}

/// `rd`, equal to `rn` one time in eight.
pub(super) fn rd_for(rng: &mut Rng, rn: u32) -> u32 {
    if rng.chance(8) {
        rn
    } else {
        reg(rng)
    }
}

/// A shift amount field value (0..=31) with the boundaries over-represented.
pub(super) fn amount(rng: &mut Rng) -> u32 {
    if rng.flag() {
        rng.pick(&[0, 1, 2, 30, 31])
    } else {
        rng.below(32)
    }
}

/// An immediate offset of `bits` bits, with 0, 1, 4 and the maximum over-represented.
fn offset(rng: &mut Rng, bits: u32) -> u32 {
    let max = (1u32 << bits) - 1;
    match rng.below(6) {
        0 => 0,
        1 => 4 & max,
        2 => max,
        3 => max & !3,
        _ => rng.below(max + 1),
    }
}

/// A register list for a block transfer: one to six registers, `pc` one time in six, `rn` in
/// the list one time in six, empty one time in 64.
pub(super) fn reg_list(rng: &mut Rng, rn: u32) -> u32 {
    if rng.chance(64) {
        return 0;
    }
    let mut list = 0;
    for _ in 0..rng.range(1, 7) {
        list |= 1 << rng.below(15);
    }
    if rng.chance(6) {
        list |= 1 << 15;
    }
    if rng.chance(6) && rn < 15 {
        list |= 1 << rn;
    }
    list
}

/// Word or byte transfer: addressing mode shared by `Ldr` and `Str`.
pub(super) fn word_transfer(rng: &mut Rng, load: bool) -> u32 {
    let rn = reg(rng);
    let rd = rd_for(rng, rn);
    let (p, u, b, w) = (rng.below(2), rng.below(2), rng.below(2), rng.below(2));
    let operand = if rng.flag() {
        1 << 25 | amount(rng) << 7 | rng.below(4) << 5 | reg(rng)
    } else {
        offset(rng, 12)
    };
    0x0400_0000
        | p << 24
        | u << 23
        | b << 22
        | w << 21
        | (load as u32) << 20
        | rn << 16
        | rd << 12
        | operand
}

/// Halfword and doubleword transfer with the `L` and `SH` bits given.
pub(super) fn half_transfer(rng: &mut Rng, load: bool, sh: u32, rd: u32, rn: u32) -> u32 {
    let (p, u, w) = (rng.below(2), rng.below(2), rng.below(2));
    let operand = if rng.flag() {
        let imm = offset(rng, 8);
        1 << 22 | (imm & 0xF0) << 4 | (imm & 0xF)
    } else {
        reg(rng)
    };
    0x0000_0090
        | p << 24
        | u << 23
        | w << 21
        | (load as u32) << 20
        | rn << 16
        | rd << 12
        | sh << 5
        | operand
}

/// A branch offset field: mostly a few words either way, sometimes a far one.
pub(super) fn branch_off(rng: &mut Rng) -> u32 {
    let words = if rng.chance(8) {
        rng.next() as i32 >> 8
    } else {
        rng.below(33) as i32 - 16
    };
    words as u32 & 0x00FF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_transfer_families_cover_every_addressing_mode_and_their_edge_offsets() {
        let mut rng = Rng::new(8);
        let mut modes = BTreeSet::new();
        let mut offs = BTreeSet::new();
        for f in [Family::Ldr, Family::Str, Family::Half, Family::Double] {
            for _ in 0..4000 {
                let w = random_of(&mut rng, f);
                modes.insert((f, w >> 24 & 1, w >> 23 & 1, w >> 21 & 1)); // P, U, W
                if matches!(f, Family::Ldr | Family::Str) && w >> 25 & 1 == 0 {
                    offs.insert(w & 0xFFF);
                }
            }
        }
        assert_eq!(
            modes.len(),
            4 * 8,
            "every P/U/W combination of the four families"
        );
        assert!(
            [0, 4, 0xFFC, 0xFFF].iter().all(|o| offs.contains(o)),
            "offsets 0, 4 and the maxima"
        );
        let mut lists = BTreeSet::new();
        for _ in 0..3000 {
            let w = random_of(&mut rng, Family::Ldm);
            lists.insert((w & 0xFFFF == 0, w >> 15 & 1, w >> 22 & 1, w >> 21 & 1));
            assert_eq!(w >> 20 & 1, 1);
        }
        assert!(
            lists.len() >= 8,
            "empty lists, pc in the list, S bit and write-back all appear: {lists:?}"
        );
        assert_eq!(random_of(&mut rng, Family::Stm) >> 20 & 1, 0);
    }

    #[test]
    fn the_shift_families_reach_every_boundary_amount() {
        let mut rng = Rng::new(9);
        let mut amounts = BTreeSet::new();
        for _ in 0..2000 {
            let w = random_of(&mut rng, Family::DpShiftImm);
            amounts.insert(w >> 7 & 31);
        }
        assert!([0, 1, 2, 30, 31].iter().all(|a| amounts.contains(a)));
        assert_eq!(amounts.len(), 32, "and every other amount");
    }
}
