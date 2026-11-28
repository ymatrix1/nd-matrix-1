//! Fixed-point multiplier effects: the product forms with and without accumulate (the 80-bit MRF
//! and MRB spill), `MRx = 0`, `SAT MRx`, `RND MRx` and the register transfers `Rn = MRx`,
//! `MRx = Rn`, each exactly as `Core::fixed_mult`, `Core::saturate_or_round_mr` and
//! `Core::mr_transfer` compute them.
//!
//! OWNER: W2-FIXED. Provenance: the interpreter source (`multiplier.rs`) is VERIFIED against the
//! CCES 2.12.1 ADSP-21489 simulator by `fixed_measured.rs` (4,096 multiplier rows: every format,
//! accumulate and subtract with extreme accumulator presets, SAT, RND, the transfers). This file
//! repeats that arithmetic on the in-place element view; it is VERIFIED equal to the interpreter
//! for every lowered multiplier opcode on edge and random operands and accumulators, MRF and MRB,
//! SIMD on and off, by `multiplier_ops_equal_the_interpreter`, for `MRx = 0`, `SAT` and `RND` on
//! the 32, 64 and 80 bit window edges and rounding ties by
//! `clear_saturate_and_round_equal_the_interpreter_at_the_window_edges`, for the transfers by
//! `multiplier_register_transfers_equal_the_interpreter` (`diff_ops.rs`), and inside the
//! multifunction forms by `dual_and_multifunction_words_equal_the_interpreter` (`diff_wide.rs`).
//!
//! ASSUMED (inherited from `multiplier.rs`): the active result register is always MRF (the MODE1
//! SRCU swap is ignored) and the integer rounding formats use the fractional rounding stage.
use super::eff::{Eff, View, MULT_FLAGS, STKY_MOS, STKY_MUS, STKY_MVS};
use super::kinds::{nxy, op8};

/// 80-bit two's complement wrap of `v` (`multiplier.rs::wrap80`).
#[inline(always)]
pub(super) fn wrap80(v: i128) -> i128 {
    (v << 48) >> 48
}

/// Round half to even at bit 32 and clear the low 32 bits (`multiplier.rs::round_mr`).
#[inline(always)]
pub(super) fn round_mr(v: i128) -> i128 {
    let low = v & 0xFFFF_FFFF;
    let mut r = v.wrapping_add(0x8000_0000);
    if low == 0x8000_0000 {
        r &= !(1i128 << 32);
    }
    wrap80(r & !0xFFFF_FFFF)
}

/// Whether `v` fits the 32-bit (integer) or 64-bit (fractional) window of the format
/// (`multiplier.rs::fits`).
#[inline(always)]
pub(super) fn fits(v: i128, frac: bool, unsigned: bool) -> bool {
    match (frac, unsigned) {
        (false, false) => (-(1i128 << 31)..1i128 << 31).contains(&v),
        (false, true) => (0..1i128 << 32).contains(&v),
        (true, false) => (-(1i128 << 63)..1i128 << 63).contains(&v),
        (true, true) => (0..1i128 << 64).contains(&v),
    }
}

/// MN, MV, MU of the multiplier into ASTAT bits 6..8 and MI cleared (`set_mult_flags`), and the
/// sticky bits the caller names.
#[inline(always)]
fn flags(e: &mut Eff, mn: bool, mv: bool, mu: bool, sticky: u32) {
    let nibble = u32::from(mn) | u32::from(mv) << 1 | u32::from(mu) << 2;
    e.astat_clear |= MULT_FLAGS;
    e.astat_set |= nibble << 6;
    e.sticky |= sticky;
}

/// Where a multiplier result goes: `Rn` for the select values 0 and 1 (the value is the low word
/// for the integer formats, the middle word for the fractional ones), the register `reg` of MRF
/// and MRB otherwise.
#[inline(always)]
fn store(e: &mut Eff, sel: u8, reg: u8, n: u8, frac: bool, v: i128) {
    if sel < 2 {
        e.write(n, if frac { (v >> 32) as u32 } else { v as u32 }, true);
    } else {
        e.mr = Some((reg, v));
    }
}

/// `Rn|MRx = [MRx +/-] Rx * Ry` for the opcode classes 1 (product), 2 (add) and 3 (subtract),
/// `Core::fixed_mult` with `cls != 0`.
#[inline(always)]
pub(super) fn product(v: &View<'_>, a: u32) -> Eff {
    let (n, x, y) = nxy(a);
    product_op(v, op8(a), n, x, y)
}

/// [`product`] with the fields given (the multifunction forms name their registers differently).
#[inline(always)]
pub(super) fn product_op(v: &View<'_>, op: u8, n: u8, x: u8, y: u8) -> Eff {
    let cls = op >> 6;
    let (sy, sx) = ((op >> 5) & 1 == 1, (op >> 4) & 1 == 1);
    let (frac, sel) = ((op >> 3) & 1 == 1, (op >> 1) & 3);
    let round = op & 1 == 1 && frac;
    let operand = |val: u32, signed: bool| {
        if signed {
            val as i32 as i128
        } else {
            val as i128
        }
    };
    let mut p = operand(v.r[usize::from(x)], sx).wrapping_mul(operand(v.r[usize::from(y)], sy));
    if frac && sx && sy {
        p <<= 1;
    }
    let reg = u8::from(sel == 1 || sel == 3);
    let acc = v.mr[usize::from(reg)];
    let mut m = wrap80(match cls {
        1 => p,
        2 => acc.wrapping_add(p),
        _ => acc.wrapping_sub(p),
    });
    if round {
        m = round_mr(m);
    }
    let mv = !fits(m, frac, !sx && !sy);
    let mu = frac && m != 0 && matches!(m >> 32, 0 | -1) && m & 0xFFFF_FFFF != 0;
    let mut e = Eff::NONE;
    flags(&mut e, m < 0, mv, mu, if mv { STKY_MOS } else { 0 });
    store(&mut e, sel, reg, n, frac, m);
    e
}

/// `MRF = 0`, `MRB = 0`, `Rn|MRx = SAT MRx (SI|UI|SF|UF)` and `Rn|MRx = RND MRx (SF|UF)`: the
/// opcode class 0 of `Core::fixed_mult` and `Core::saturate_or_round_mr`.
#[inline(always)]
pub(super) fn clear_sat_round(v: &View<'_>, a: u32) -> Eff {
    let (op, (n, _, _)) = (op8(a), nxy(a));
    let mut e = Eff::NONE;
    if op == 0x14 || op == 0x16 {
        // VERIFIED in the dual-memory form (census diff, 12 cases, `multiplier.rs`): clearing an
        // MR register clears MN, MV, MU, MI
        flags(&mut e, false, false, false, 0);
        e.mr = Some((u8::from(op == 0x16), 0));
        return e;
    }
    let (sel, unsigned, frac) = ((op >> 1) & 3, op & 1 == 0, (op >> 3) & 1 == 1);
    let reg = u8::from(sel == 1 || sel == 3);
    let m = v.mr[usize::from(reg)];
    let (out, mv) = if op >> 4 == 0 {
        let (lo, hi) = match (frac, unsigned) {
            (false, false) => (-(1i128 << 31), (1i128 << 31) - 1),
            (false, true) => (0, (1i128 << 32) - 1),
            (true, false) => (-(1i128 << 63), (1i128 << 63) - 1),
            (true, true) => (0, (1i128 << 64) - 1),
        };
        let c = if (lo..=hi).contains(&m) {
            m
        } else if !unsigned && m < 0 {
            lo
        } else {
            hi
        };
        (c, false)
    } else {
        // `RND` (`op >> 4 == 1` with the fractional bit; the lowering admits nothing else)
        let r = round_mr(m);
        (
            r,
            !fits(r, true, unsigned)
                || !fits(m, true, unsigned)
                || (m < 0 && m.wrapping_add(0x8000_0000) >= 0),
        )
    };
    let mu = frac && out != 0 && matches!(out >> 32, 0 | -1) && out & 0xFFFF_FFFF != 0;
    let sticky = if mv { STKY_MVS } else { 0 } | if mu { STKY_MUS } else { 0 };
    flags(&mut e, out < 0, mv, mu, sticky);
    store(&mut e, sel, reg, n, frac, out);
    e
}

/// `Rn = MRxF/B` (read) and `MRxF/B = Rn` (write) of the multifunction word `a` with bits 21..17
/// clear (`Core::mr_transfer`). A read writes the register WITHOUT dropping its float extension
/// and a transfer changes no flag, as in the interpreter.
#[inline(always)]
pub(super) fn transfer(v: &View<'_>, a: u32) -> Eff {
    let op = op8(a);
    let (n, _, _) = nxy(a);
    let (field, background, write) = (op & 3, (op >> 2) & 1 == 1, (op >> 4) & 1 == 1);
    let reg = u8::from(background);
    let m = v.mr[usize::from(reg)];
    let mut e = Eff::NONE;
    if !write {
        e.write(
            n,
            match field {
                0 => m as u32,
                1 => (m >> 32) as u32,
                _ => (((m >> 64) as i16) as i32) as u32,
            },
            false,
        );
        return e;
    }
    let rn = v.r[usize::from(n)];
    let low = m & 0xFFFF_FFFF;
    let low64 = m & 0xFFFF_FFFF_FFFF_FFFF;
    let new = match field {
        0 => (m & !0xFFFF_FFFFi128) | i128::from(rn),
        1 => i128::from(rn as i32) << 32 | low,
        _ => i128::from(rn as u16 as i16) << 64 | low64,
    };
    e.mr = Some((reg, wrap80(new)));
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::exec::h_fixed::eff::view;
    use crate::core::uop::exec::h_fixed::kinds::word;
    use crate::core::Core;

    /// A core with R1, R2 and MRF set.
    fn core(r1: u32, r2: u32, mrf: i128) -> Core {
        let mut c = core_with(&[0x0001]);
        (c.r[1], c.r[2], c.mr[0]) = (r1, r2, mrf);
        c
    }

    /// The multiplier word `R3|MRx = R1 * R2` of opcode `op`.
    fn mult(c: &Core, op: u8) -> Eff {
        product(&view(c, false), 1 << 20 | word(op, 3, 1, 2))
    }

    fn class0(c: &Core, op: u8, n: u8) -> Eff {
        clear_sat_round(&view(c, false), 1 << 20 | word(op, n, 0, 0))
    }

    #[test]
    fn only_the_signed_fractional_product_is_shifted_left() {
        let c = core(1, 1, 0);
        assert_eq!(mult(&c, 0x78 | 4).mr, Some((0, 2)), "SSF");
        assert_eq!(mult(&c, 0x68 | 4).mr, Some((0, 1)), "SUF");
        assert_eq!(mult(&c, 0x70 | 4).mr, Some((0, 1)), "SSI");
    }

    #[test]
    fn the_fractional_signed_overflow_sets_mv_and_mos_and_does_not_saturate() {
        let c = core(0x8000_0000, 0x8000_0000, 0);
        let e = mult(&c, 0x78);
        assert_eq!((e.wr[0].v, e.wr[0].n, e.sticky), (0x8000_0000, 3, STKY_MOS));
        assert_eq!(e.astat_set, 0b010 << 6, "MV only");
        assert_eq!(e.astat_clear, MULT_FLAGS);
        assert!(e.wr[0].clear_lo, "Rn loses its float extension");
    }

    #[test]
    fn a_result_select_of_two_or_three_spills_into_the_80_bit_register() {
        let acc = (1i128 << 79) - 1;
        let c = core(0x7FFF_FFFF, 0x7FFF_FFFF, acc);
        // class 2 (MRF + product), SSF, select 2 = MRF: the sum wraps at 80 bits
        let e = mult(&c, 0xBC);
        let p = (0x7FFF_FFFFi128 * 0x7FFF_FFFFi128) << 1;
        assert_eq!(e.mr, Some((0, acc + p - (1i128 << 80))));
        assert_eq!(e.nw, 0, "no register written");
        assert_eq!(
            e.astat_set & (1 << 6),
            1 << 6,
            "MN: the wrapped value is negative"
        );
        assert_eq!(e.sticky, STKY_MOS);
        // select 3 reads and writes MRB
        let mut c = core(3, 5, 0);
        c.mr[1] = 100;
        assert_eq!(mult(&c, 0xB6).mr, Some((1, 115)), "MRB = MRB + SSI");
        assert_eq!(mult(&c, 0xF6).mr, Some((1, 85)), "MRB = MRB - SSI");
    }

    #[test]
    fn the_rounding_formats_round_half_to_even_and_the_integer_ones_do_not() {
        let c = core(1, 0x4000_0000, 0);
        let e = mult(&c, 0x79);
        assert_eq!(e.wr[0].v, 0, "SSFR: 2^31 rounds to even (0)");
        assert_eq!(mult(&core(3, 5, 0), 0x71).wr[0].v, 15, "SSIR is SSI");
        assert_eq!(round_mr(0x1_8000_0000), 0x2_0000_0000);
        assert_eq!(round_mr(0x2_8000_0000), 0x2_0000_0000, "ties go to even");
        assert!(fits(-(1i128 << 31), false, false) && !fits(1i128 << 31, false, false));
        assert!(fits((1i128 << 64) - 1, true, true) && !fits(-1, true, true));
    }

    #[test]
    fn a_small_fractional_result_sets_mu_and_latches_nothing() {
        let e = mult(&core(1, 1, 0), 0x6C);
        assert_eq!(e.astat_set, 0b100 << 6, "MU only");
        assert_eq!(e.sticky, 0, "the multiplier MU does not latch a sticky bit");
    }

    #[test]
    fn clearing_mr_clears_the_multiplier_flags_and_the_register() {
        let c = core(0, 0, 0x1_2345_6789);
        let e = class0(&c, 0x14, 0);
        assert_eq!(
            (e.mr, e.astat_set, e.astat_clear, e.sticky),
            (Some((0, 0)), 0, MULT_FLAGS, 0)
        );
        assert_eq!(class0(&c, 0x16, 0).mr, Some((1, 0)));
    }

    #[test]
    fn saturate_clamps_to_the_window_of_the_format() {
        let c = core(0, 0, 1i128 << 70);
        let e = class0(&c, 0x09, 3); // R3 = SAT MRF (SF)
        assert_eq!((e.wr[0].n, e.wr[0].v, e.sticky), (3, 0x7FFF_FFFF, 0));
        assert_eq!(
            class0(&c, 0x09 | 4, 3).mr,
            Some((0, 0x7FFF_FFFF_FFFF_FFFF)),
            "MRF = SAT MRF"
        );
        let c = core(0, 0, 0x1234);
        assert_eq!(class0(&c, 0x01, 4).wr[0].v, 0x1234, "in range: unchanged");
        let c = core(0, 0, -(1i128 << 70));
        assert_eq!(
            class0(&c, 0x00, 4).wr[0].v,
            0xFFFF_FFFF,
            "unsigned: the maximum"
        );
        assert_eq!(
            class0(&c, 0x01, 4).wr[0].v,
            0x8000_0000,
            "signed: the minimum"
        );
        let c = core(0, 0, -5);
        assert_eq!(
            class0(&c, 0x01, 4).astat_set,
            1 << 6,
            "MN for a negative result"
        );
    }

    #[test]
    fn round_latches_mvs_when_the_result_leaves_the_window() {
        let c = core(0, 0, 3 << 31); // 1.5 in MR1
        let e = class0(&c, 0x19, 5); // R5 = RND MRF (SF)
        assert_eq!((e.wr[0].n, e.wr[0].v, e.sticky), (5, 2, 0));
        let c = core(0, 0, (1i128 << 63) - 1);
        let e = class0(&c, 0x19, 5);
        assert_eq!(
            e.sticky & STKY_MVS,
            STKY_MVS,
            "rounding up past the 64-bit window"
        );
        assert_eq!(e.astat_set & (1 << 7), 1 << 7, "MV");
    }

    #[test]
    fn mr_transfers_read_the_three_fields_and_write_with_sign_extension() {
        let mut c = core(0, 0, 0);
        c.mr[0] = (-5i128 << 64) | 0x1234_5678_9ABC_DEF0;
        let v = view(&c, false);
        let read = |field: u8| transfer(&v, word(field, 4, 0, 0)).wr[0].v;
        assert_eq!(read(0), 0x9ABC_DEF0);
        assert_eq!(read(1), 0x1234_5678);
        assert_eq!(read(2), (-5i32) as u32, "MR2 is sign extended from 16 bits");
        assert!(
            !transfer(&v, word(0, 4, 0, 0)).wr[0].clear_lo,
            "the extension stays"
        );
        c.r[4] = 0x8000_8001;
        let low = c.mr[0] & 0xFFFF_FFFF;
        let v = view(&c, false);
        let w1 = transfer(&v, word(0x11, 4, 0, 0)).mr.unwrap();
        assert_eq!(w1.0, 0);
        assert_eq!(
            w1.1 >> 32,
            -0x7FFF_7FFFi128,
            "a write to MR1 sign extends into MR2"
        );
        assert_eq!(w1.1 & 0xFFFF_FFFF, low, "MR0 is kept");
        let w0 = transfer(&v, word(0x10, 4, 0, 0)).mr.unwrap().1;
        assert_eq!(
            w0,
            (c.mr[0] & !0xFFFF_FFFFi128) | 0x8000_8001,
            "MR0 write keeps the rest"
        );
        let w2 = transfer(&v, word(0x12 | 4, 4, 0, 0)).mr.unwrap();
        assert_eq!(w2.0, 1, "bit 2 selects the background register");
        assert_eq!(
            w2.1 >> 64,
            -32767,
            "MR2 takes the low 16 bits of Rn, signed"
        );
        let e = transfer(&v, word(0x11, 4, 0, 0));
        assert_eq!(
            (e.astat_clear, e.astat_set, e.nw, e.sticky),
            (0, 0, 0, 0),
            "no flag, no register"
        );
    }
}
