//! Fixed-point ALU effects: add, subtract, average, compare, pass, negate, increment,
//! decrement, absolute value, logic, minimum, maximum and clip, each exactly as `Core::fixed_alu`
//! computes it (`fixed_alu.rs`: VERIFIED against the CCES 2.12.1 ADSP-21489 simulator by
//! `fixed_measured.rs`, flags, carry, saturation and the half-to-even average).
//!
//! OWNER: W2-FIXED. [`add`] and [`sub`] are shared with the dual add/subtract and the
//! multifunction forms (`multi.rs`). Provenance of this file: every result and flag below is
//! VERIFIED equal to the interpreter for every opcode on edge and random operands, SIMD on and
//! off, by `alu_ops_equal_the_interpreter`, and for the saturated and wrapped results at the
//! edges of the 32 bit range by `saturation_and_wraparound_give_the_interpreters_values`
//! (`diff_ops.rs`); the semantics themselves are those of the interpreter source cited above.
use super::eff::{AluFlags, Eff, View, AF, STKY_AOS};
use super::kinds::nxy;

/// `Rn = r` with the flags of [`AluFlags::of`] and AOS when `av` (`Core::fixed_result`: the float
/// extension of the register is dropped and AI is cleared; AOS is what `note_arith_exception`
/// raises).
#[inline(always)]
fn result(n: u8, r: u32, ac: bool, av: bool, as_: bool) -> Eff {
    let mut e = Eff::NONE;
    e.write(n, r, true);
    e.alu = Some(AluFlags::of(r, ac, av, as_));
    e.sticky = if av { STKY_AOS } else { 0 };
    e
}

/// The saturated value of an overflowing result with ALUSAT set (`Core::sat_result`).
#[inline(always)]
fn limited(sat: bool, av: bool, exact: i64, plain: u32) -> u32 {
    if av && sat {
        if exact > 0 {
            0x7FFF_FFFF
        } else {
            0x8000_0000
        }
    } else {
        plain
    }
}

/// `Rn = x + y + cin` (`Core::fixed_add`).
#[inline(always)]
pub(super) fn add(sat: bool, n: u8, x: u32, y: u32, cin: u64) -> Eff {
    let t = u64::from(x) + u64::from(y) + cin;
    let exact = i64::from(x as i32) + i64::from(y as i32) + cin as i64;
    let av = i32::try_from(exact).is_err();
    result(
        n,
        limited(sat, av, exact, t as u32),
        t >> 32 != 0,
        av,
        false,
    )
}

/// `Rn = x + !y + cin`, i.e. `x - y + cin - 1` (`Core::fixed_sub`).
#[inline(always)]
pub(super) fn sub(sat: bool, n: u8, x: u32, y: u32, cin: u64) -> Eff {
    let t = u64::from(x) + u64::from(!y) + cin;
    let exact = i64::from(x as i32) - i64::from(y as i32) + cin as i64 - 1;
    let av = i32::try_from(exact).is_err();
    result(
        n,
        limited(sat, av, exact, t as u32),
        t >> 32 != 0,
        av,
        false,
    )
}

/// `Rn = (x + y) / 2`, half to even, AC the carry of `x + y + rounding` (`0x09`).
#[inline(always)]
fn average(n: u8, rx: u32, ry: u32) -> Eff {
    let a = i64::from(rx as i32) + i64::from(ry as i32);
    let rnd = i64::from(a & 1 == 1 && (a >> 1) & 1 == 1);
    let t = u64::from(rx) + u64::from(ry) + rnd as u64;
    result(n, ((a + rnd) >> 1) as u32, t >> 32 != 0, false, false)
}

/// `COMP` / `COMPU`: flags from `Rx` against `Ry`, `Rx > Ry` shifted into CACC, no result
/// register (`0x0A`, `0x0B`).
#[inline(always)]
fn compare(op: u8, rx: u32, ry: u32) -> Eff {
    let (less, greater) = if op == 0x0A {
        ((rx as i32) < (ry as i32), (rx as i32) > (ry as i32))
    } else {
        (rx < ry, rx > ry)
    };
    let mut e = Eff::NONE;
    e.cacc = Some(greater);
    e.alu = Some(AluFlags {
        az: rx == ry,
        an: less,
        av: false,
        ac: false,
        as_: false,
    });
    e
}

/// `Rn = ABS Rx`: AS is the sign of the operand, AV when it is the most negative value.
#[inline(always)]
fn abs(sat: bool, n: u8, rx: u32) -> Eff {
    let exact = i64::from(rx as i32).abs();
    let av = exact >= 1 << 31;
    result(
        n,
        limited(sat, av, exact, exact as u32),
        false,
        av,
        (rx as i32) < 0,
    )
}

/// A result that only sets AZ and AN (`Core::fixed_plain`).
#[inline(always)]
fn plain(n: u8, r: u32) -> Eff {
    result(n, r, false, false, false)
}

/// The effect of fixed ALU opcode `op` with the compute word `a` (`Core::fixed_alu`). Opcodes
/// outside the modelled list cannot reach this function: the lowering does not emit them.
#[inline(always)]
pub(super) fn effect(v: &View<'_>, op: u8, a: u32) -> Eff {
    let (n, x, y) = nxy(a);
    let (rx, ry) = (v.r[usize::from(x)], v.r[usize::from(y)]);
    let ci = u64::from(v.f.ac);
    let mut e = match op {
        0x01 => add(v.sat, n, rx, ry, 0),
        0x02 => sub(v.sat, n, rx, ry, 1),
        0x05 => add(v.sat, n, rx, ry, ci),
        0x06 => sub(v.sat, n, rx, ry, ci),
        0x09 => average(n, rx, ry),
        0x0A | 0x0B => compare(op, rx, ry),
        0x21 => plain(n, rx),
        0x22 => sub(v.sat, n, 0, rx, 1),
        0x25 => add(v.sat, n, rx, 0, ci),
        0x26 => sub(v.sat, n, rx, 0, ci),
        0x29 => add(v.sat, n, rx, 1, 0),
        0x2A => sub(v.sat, n, rx, 1, 1),
        0x30 => abs(v.sat, n, rx),
        0x40 => plain(n, rx & ry),
        0x41 => plain(n, rx | ry),
        0x42 => plain(n, rx ^ ry),
        0x43 => plain(n, !rx),
        0x61 => plain(n, (rx as i32).min(ry as i32) as u32),
        0x62 => plain(n, (rx as i32).max(ry as i32) as u32),
        _ => {
            // 0x63 CLIP: `Rx` limited to the magnitude of `Ry`
            let lim = i64::from(ry as i32).abs();
            plain(n, i64::from(rx as i32).clamp(-lim, lim) as u32)
        }
    };
    // a fixed ALU operation clears AF (ASSUMED by `alu.rs`, every simulator probe started from
    // ASTAT = 0; the interpreter does it for every opcode of the unit)
    e.astat_clear |= AF;
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    fn run(op: u8, rx: u32, ry: u32, sat: bool, ac: bool) -> Eff {
        let mut c = core_with(&[0x0001]);
        c.r[1] = rx;
        c.r[2] = ry;
        c.flags.ac = ac;
        if sat {
            c.mode1 = crate::core::float::MODE1_ALUSAT;
        }
        effect(
            &super::super::eff::view(&c, false),
            op,
            super::super::kinds::word(op, 3, 1, 2),
        )
    }

    #[test]
    fn add_overflow_sets_av_and_the_sticky_bit_and_alusat_saturates() {
        let e = run(0x01, 0x7FFF_FFFF, 1, false, false);
        assert_eq!((e.wr[0].v, e.sticky), (0x8000_0000, STKY_AOS));
        let f = e.alu.unwrap();
        assert!(f.av && f.an && !f.ac && !f.az);
        let e = run(0x01, 0x7FFF_FFFF, 1, true, false);
        assert_eq!(
            (e.wr[0].v, e.sticky),
            (0x7FFF_FFFF, STKY_AOS),
            "saturated, still AV"
        );
        let e = run(0x01, 0x8000_0000, 0x8000_0000, true, false);
        assert_eq!(e.wr[0].v, 0x8000_0000);
        assert!(e.alu.unwrap().ac && !e.alu.unwrap().az);
    }

    #[test]
    fn subtract_carries_when_the_operands_are_equal_and_uses_the_carry_in() {
        let e = run(0x02, 5, 5, false, false);
        let f = e.alu.unwrap();
        assert!(f.az && f.ac && !f.av, "x - x carries");
        assert_eq!(e.sticky, 0);
        // with carry-in: x + !y + ci
        assert_eq!(run(0x06, 10, 3, false, false).wr[0].v, 6);
        assert_eq!(run(0x06, 10, 3, false, true).wr[0].v, 7);
        assert_eq!(run(0x05, 10, 3, false, true).wr[0].v, 14);
    }

    #[test]
    fn average_rounds_half_to_even_and_compare_shifts_cacc() {
        assert_eq!(run(0x09, 0, 1, false, false).wr[0].v, 0);
        let e = run(0x09, 0, 0xFFFF_FFFF, false, false);
        assert!(e.alu.unwrap().az && e.alu.unwrap().ac);
        let e = run(0x0A, 5, 3, false, false);
        assert_eq!((e.nw, e.cacc, e.alu.unwrap().an), (0, Some(true), false));
        let e = run(0x0B, 0xFFFF_FFFF, 3, false, false);
        assert_eq!((e.cacc, e.alu.unwrap().an), (Some(true), false), "unsigned");
        let e = run(0x0A, 0xFFFF_FFFF, 3, false, false);
        assert_eq!((e.cacc, e.alu.unwrap().an), (Some(false), true), "signed");
    }

    #[test]
    fn every_alu_effect_clears_af_and_drops_the_float_extension_of_its_result() {
        for op in super::super::kinds::ALU_OPS {
            let e = run(op, 0x1234_5678, 0x0F0F_0F0F, false, false);
            assert_eq!(e.astat_clear & AF, AF, "op {op:#x}");
            assert!(e.alu.is_some());
            if !matches!(op, 0x0A | 0x0B) {
                assert!(
                    e.nw == 1 && e.wr[0].clear_lo && e.wr[0].n == 3,
                    "op {op:#x}"
                );
            }
        }
    }

    #[test]
    fn abs_neg_min_max_and_clip() {
        let e = run(0x30, 0x8000_0000, 0, false, false);
        assert!(e.alu.unwrap().av && e.alu.unwrap().as_);
        assert_eq!((e.wr[0].v, e.sticky), (0x8000_0000, STKY_AOS));
        assert_eq!(run(0x30, 0x8000_0000, 0, true, false).wr[0].v, 0x7FFF_FFFF);
        assert_eq!(run(0x22, 7, 0, false, false).wr[0].v as i32, -7);
        assert_eq!(
            run(0x61, 7, (-3i32) as u32, false, false).wr[0].v as i32,
            -3
        );
        assert_eq!(run(0x62, 7, (-3i32) as u32, false, false).wr[0].v, 7);
        assert_eq!(run(0x63, 7, (-3i32) as u32, false, false).wr[0].v, 3);
        assert_eq!(run(0x29, 0xFFFF_FFFF, 0, false, false).wr[0].v, 0);
        assert_eq!(run(0x2A, 0, 0, false, false).wr[0].v, 0xFFFF_FFFF);
    }
}
