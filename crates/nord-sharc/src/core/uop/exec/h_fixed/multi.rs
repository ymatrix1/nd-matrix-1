//! The instructions with more than one fixed-point result: the dual add/subtract
//! `Ra = Rx + Ry, Rs = Rx - Ry`, the multiplier plus ALU multifunction (`MF2`) and the multiplier
//! with an add and a subtract (`MF3`), each exactly as `Core::dual_add_sub` and
//! `Core::multifunction` compute them.
//!
//! OWNER: W2-FIXED. Both halves read the register values from before the instruction (the
//! effects below are computed from one [`View`], nothing is written between them). The writes are
//! ordered as the interpreter commits them, so a register named twice takes the later value:
//! multiplier result, then sum, then difference. The ALU flags are those of the ALU half (MF2) or
//! the OR of the sum's and the difference's (dual, MF3); the multiplier half sets MN, MV, MU, MI.
//! AF is cleared by every one of these forms (they all run an ALU operation).
//!
//! Provenance: VERIFIED equal to the interpreter for random dual words and for the multifunction
//! word space (every lowered op6 of the multiplier plus ALU form, the triple form, random operand
//! fields, SIMD on and off, ALUSAT on and off) by
//! `dual_and_multifunction_words_equal_the_interpreter` (`diff_wide.rs`); the semantics are those
//! of `multifunction.rs` (VERIFIED against the simulator: 64 operand pairs of the dual form, 5
//! triple-operation cases, `simd_measured.rs` and the census diff).
use super::alu::{add, effect as alu_effect, sub};
use super::eff::{Eff, View, AF};
use super::kinds::word;
use super::mult::product_op;

/// The multiplier operation of each multiplier group of the `MF2` forms (`op6 >> 2` = 1..=5):
/// `Rm = Rxm * Rym (SSFR)`, `MRF = MRF + ... (SSF)`, `Rm = MRF + ... (SSFR)`,
/// `MRF = MRF - ... (SSF)`, `Rm = MRF - ... (SSFR)`. VERIFIED: `FIXED_MULT_OPS` of
/// `multifunction.rs`, checked by `multifunction_forms_follow_the_interpreters_tables` below.
pub const MF2_MULT: [u8; 5] = [0x79, 0xBC, 0xB9, 0xFC, 0xF9];
/// The ALU operation of each low pair of bits of `op6`: add, subtract, average. VERIFIED:
/// `FIXED_ALU_OPS` of `multifunction.rs`.
pub const MF2_ALU: [u8; 3] = [0x01, 0x02, 0x09];
/// The multiplier operation of the triple form: `Rm = Rxm * Rym (SSFR)`.
pub const MF3_MULT: u8 = 0x79;

/// The multifunction operand registers: `Rxm` is R0..R3, `Rym` R4..R7, `Rxa` R8..R11, `Rya`
/// R12..R15 (`multifunction.rs`).
#[inline(always)]
const fn operands(a: u32) -> (u8, u8, u8, u8) {
    (
        ((a >> 6) & 3) as u8,
        4 + ((a >> 4) & 3) as u8,
        8 + ((a >> 2) & 3) as u8,
        12 + (a & 3) as u8,
    )
}

/// `sum` and `diff` as one effect: the writes in that order, the ALU flags ORed, the sticky bits
/// ORed and AF cleared.
#[inline(always)]
fn both(mut first: Eff, sum: &Eff, diff: &Eff) -> Eff {
    first.then_writes(sum);
    first.then_writes(diff);
    if let (Some(s), Some(d)) = (sum.alu, diff.alu) {
        first.alu = Some(s.or(d));
    }
    first.astat_clear |= AF;
    first
}

/// `Ra = Rx + Ry, Rs = Rx - Ry` (the fixed dual add/subtract, opcodes `0x70..0x7F`):
/// `Rs` is bits 15..12, `Ra` bits 11..8.
#[inline(always)]
pub(super) fn dual(v: &View<'_>, a: u32) -> Eff {
    let (rs, ra, x, y) = (
        ((a >> 12) & 15) as u8,
        ((a >> 8) & 15) as u8,
        (a >> 4) & 15,
        a & 15,
    );
    let (rx, ry) = (v.r[x as usize], v.r[y as usize]);
    both(
        Eff::NONE,
        &add(v.sat, ra, rx, ry, 0),
        &sub(v.sat, rs, rx, ry, 1),
    )
}

/// The multiplier plus ALU form: `op6` = bits 21..16 in `4..=22` with a low pair of 0..=2.
#[inline(always)]
pub(super) fn mf2(v: &View<'_>, a: u32) -> Eff {
    let op6 = (a >> 16) & 63;
    let (xm, ym, xa, ya) = operands(a);
    let (rm, ra) = (((a >> 12) & 15) as u8, ((a >> 8) & 15) as u8);
    // the lowering admits only groups 1..=5 and low pairs 0..=2; the modulo keeps an index that
    // never occurs in range instead of panicking inside an `extern "C"` handler
    let mult_op = MF2_MULT[(op6 >> 2).wrapping_sub(1) as usize % MF2_MULT.len()];
    let mut e = product_op(v, mult_op, rm, xm, ym);
    let alu_op = MF2_ALU[(op6 & 3) as usize % MF2_ALU.len()];
    let alu = alu_effect(v, alu_op, word(alu_op, ra, xa, ya));
    e.then_writes(&alu);
    e.alu = alu.alu;
    e.astat_clear |= alu.astat_clear;
    e
}

/// The triple form `Rm = Rxm * Rym, Ra = Rxa + Rya, Rs = Rxa - Rya` (bit 21 set, bit 20 clear):
/// `Rs` is bits 19..16, `Rm` bits 15..12, `Ra` bits 11..8.
#[inline(always)]
pub(super) fn mf3(v: &View<'_>, a: u32) -> Eff {
    let rs = ((a >> 16) & 15) as u8;
    let (rm, ra) = (((a >> 12) & 15) as u8, ((a >> 8) & 15) as u8);
    let (xm, ym, xa, ya) = operands(a);
    let (rx, ry) = (v.r[usize::from(xa)], v.r[usize::from(ya)]);
    both(
        product_op(v, MF3_MULT, rm, xm, ym),
        &add(v.sat, ra, rx, ry, 0),
        &sub(v.sat, rs, rx, ry, 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::exec::h_fixed::eff::{view, MULT_FLAGS, STKY_AOS};
    use crate::core::Core;

    fn core() -> Core {
        let mut c = core_with(&[0x0001]);
        for (k, r) in c.r.iter_mut().enumerate() {
            *r = 0x100 + k as u32;
        }
        c
    }

    /// The multifunction word of `op6` with the register fields set.
    fn mf_word(op6: u32, rm: u32, ra: u32, f: u32) -> u32 {
        1 << 22 | op6 << 16 | rm << 12 | ra << 8 | f
    }

    #[test]
    fn dual_writes_the_sum_then_the_difference_and_ors_the_flags() {
        let mut c = core();
        (c.r[0], c.r[1]) = (10, 3);
        let e = dual(&view(&c, false), (0x7 << 16) | (2 << 12) | (3 << 8) | 1);
        assert_eq!(
            (e.nw, e.wr[0].n, e.wr[0].v, e.wr[1].n, e.wr[1].v),
            (2, 3, 13, 2, 7)
        );
        assert!(e.wr[0].clear_lo && e.wr[1].clear_lo);
        assert_eq!(e.astat_clear, AF);
        // a sum that is positive with a difference that is negative: AN of either
        (c.r[9], c.r[13]) = (0x3FC0_0000, 0x4000_0000);
        let e = dual(
            &view(&c, false),
            (0x7 << 16) | (8 << 12) | (4 << 8) | (9 << 4) | 13,
        );
        assert_eq!((e.wr[0].v, e.wr[1].v), (0x7FC0_0000, 0xFFC0_0000));
        assert!(e.alu.unwrap().an && !e.alu.unwrap().az);
        // the same register for both results: the difference wins, in the write order
        let e = dual(&view(&c, false), (0x7 << 16) | (5 << 12) | (5 << 8) | 1);
        assert_eq!((e.wr[0].n, e.wr[1].n), (5, 5));
    }

    #[test]
    fn dual_overflow_raises_aos_from_either_half() {
        let mut c = core();
        (c.r[0], c.r[1]) = (0x7FFF_FFFF, (-1i32) as u32);
        let e = dual(&view(&c, false), (0x7 << 16) | (2 << 12) | (3 << 8) | 1);
        assert_eq!(e.sticky, STKY_AOS, "the difference overflows");
        assert!(e.alu.unwrap().av);
    }

    #[test]
    fn mf2_runs_the_multiplier_and_the_alu_from_the_original_registers() {
        // MRF = MRF + R0 * R4 (SSF), R9 = R8 + R12
        let mut c = core();
        (c.r[0], c.r[4], c.r[8], c.r[12]) = (0x4000_0000, 0x4000_0000, 5, 6);
        let e = mf2(&view(&c, false), mf_word(0b001000, 0, 9, 0));
        assert_eq!((e.nw, e.wr[0].n, e.wr[0].v), (1, 9, 11));
        assert_eq!(e.mr.map(|(k, v)| (k, v >> 32)), Some((0, 0x2000_0000)));
        assert_eq!(e.astat_clear, MULT_FLAGS | AF);
        // Rm = Rxm * Rym (SSFR), Ra = Rxa - Rya: the multiplier writes first
        let e = mf2(&view(&c, false), mf_word(0b000101, 1, 2, 0));
        assert_eq!(
            (e.wr[0].n, e.wr[0].v, e.wr[1].n, e.wr[1].v),
            (1, 0x2000_0000, 2, 5u32.wrapping_sub(6))
        );
        // the multiplier and the ALU both name R7: the ALU value is committed last
        let e = mf2(&view(&c, false), mf_word(0b000100, 7, 7, 0));
        assert_eq!((e.wr[0].n, e.wr[1].n), (7, 7));
    }

    #[test]
    fn multifunction_forms_follow_the_interpreters_tables() {
        // op6 = group << 2 | low: groups 1..=5 pick the multiplier, low 0..=2 the ALU
        let mut c = core();
        (c.r[8], c.r[12]) = (100, 40);
        for (group, mult) in MF2_MULT.iter().enumerate() {
            let sel = (mult >> 1) & 3;
            for (low, alu) in MF2_ALU.iter().enumerate() {
                let e = mf2(
                    &view(&c, false),
                    mf_word(((group as u32 + 1) << 2) | low as u32, 0, 9, 0),
                );
                let alu_value = match alu {
                    0x01 => 140,
                    0x02 => 60,
                    _ => 70,
                };
                let last = e.wr[usize::from(e.nw) - 1];
                assert_eq!((last.n, last.v), (9, alu_value), "group {group} low {low}");
                assert_eq!(
                    e.mr.is_some(),
                    sel >= 2,
                    "group {group}: MRF result iff select >= 2"
                );
            }
        }
    }

    #[test]
    fn mf3_orders_the_multiplier_then_the_sum_then_the_difference() {
        // Rm = R0 * R4 (SSFR), Ra = R8 + R12, Rs = R8 - R12 with Rs = 3, Rm = 5, Ra = 6
        let mut c = core();
        (c.r[0], c.r[4], c.r[8], c.r[12]) = (0x4000_0000, 0x4000_0000, 9, 4);
        let e = mf3(
            &view(&c, false),
            1 << 22 | 1 << 21 | (3 << 16) | (5 << 12) | (6 << 8),
        );
        let w: Vec<(u8, u32)> = e.wr[..3].iter().map(|w| (w.n, w.v)).collect();
        assert_eq!(w, [(5, 0x2000_0000), (6, 13), (3, 5)]);
        assert_eq!(e.astat_clear, MULT_FLAGS | AF);
        assert!(e.alu.is_some() && e.mr.is_none());
    }
}
