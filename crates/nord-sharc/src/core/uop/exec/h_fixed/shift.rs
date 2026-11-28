//! Shifter effects: the register form `Rn = <op> Rx BY Ry` and the shift-immediate form
//! `Rn = <op> Rx BY <imm12>`, for the 21 shifter operations that do not touch the bit FIFO, each
//! exactly as `Core::shifter_unit` computes it.
//!
//! OWNER: W2-FIXED. Provenance: the operation semantics are those of `shifter.rs` (VERIFIED
//! against the CCES 2.12.1 ADSP-21489 simulator by its census diff, 600+ cases, see the doc of
//! `shifter_op`); the bit helpers (`shift`, `deposit`, `extract`, `fpack`, `funpack`) are the v1
//! ones, called as they are. This file is VERIFIED equal to the interpreter for every modelled
//! opcode on random operands, SIMD on and off, by `shifter_ops_equal_the_interpreter`, on 6 edge
//! values times 14 counts and field descriptors by `shifter_edges_equal_the_interpreter`, and for
//! the immediate form by `shift_immediate_equals_the_interpreter` (`diff_wide.rs`). The `0x08`
//! rotate and the OR forms read the old `Rn` from the same element.
//!
//! Every operation clears ASTAT SF (bit 14) and sets all three shifter flags SZ, SV, SS. A result
//! is a raw write to `Rn`: the float extension byte is kept, as in the interpreter (the ALU and
//! the multiplier drop it, the shifter does not). `BTST` writes no register.
use super::eff::{Eff, View, SF};
use super::kinds::nxy;
use crate::core::shifter_bits::{deposit, extract, fpack, funpack, shift};

/// The shifter opcodes this package lowers, ascending. VERIFIED: the arms of
/// `Core::shifter_unit` without the bit FIFO ones (`0x50`, `0x58`, `0x70`, `0x74`, `0x7C`),
/// checked against the interpreter over all 256 opcodes by `the_shifter_list_is_the_interpreters`
/// (`lower/fixed/valid.rs`).
pub const SHIFT_OPS: [u8; 21] = [
    0x00, 0x04, 0x08, 0x20, 0x24, 0x40, 0x44, 0x48, 0x4C, 0x64, 0x6C, 0x80, 0x84, 0x88, 0x8C, 0x90,
    0x94, 0xC0, 0xC4, 0xC8, 0xCC,
];

/// The shift amount of an immediate form: the operations that shift or rotate take the low 8 bits
/// of the 12-bit immediate as a signed count, the others the whole 12 bits (`shift_imm_params`).
/// VERIFIED (simulator census, 75 shift-immediate shapes x 3 states): bits 11..8 of the immediate
/// are ignored for `LSHIFT`, `ASHIFT`, `ROT` and their OR forms.
#[inline(always)]
pub(super) const fn imm_ry(op: u8, data: u32) -> u32 {
    if matches!(op, 0x00 | 0x04 | 0x08 | 0x20 | 0x24) {
        (data as u8 as i8 as i32) as u32
    } else {
        data
    }
}

/// The three shifter flags and the result of `op`, in the order `(sz, sv, ss, result)`; the
/// result is `None` for `BTST`.
#[inline(always)]
fn unit(v: &View<'_>, op: u8, n: u8, rx: u32, ry: u32) -> (bool, bool, bool, Option<u32>) {
    let old = v.r[usize::from(n)];
    let (len, pos) = ((ry >> 6) & 63, ry & 63);
    let bit = ry as i32;
    let bit_ok = (0..32).contains(&bit);
    match op {
        0x00 | 0x20 | 0x04 | 0x24 => {
            let arith = op & 4 != 0;
            let r = shift(rx, ry as i32, arith) | if op & 0x20 != 0 { old } else { 0 };
            // VERIFIED (simulator): LSHIFT sets SV for any left shift, ASHIFT only when a
            // significant bit was lost
            let left = (ry as i32) > 0;
            let sv = left
                && (!arith
                    || shift(rx, ry as i32, true) as i32 >> ry.min(31) != rx as i32
                    || (ry >= 32 && (rx != 0 || op & 0x20 != 0)));
            (r == 0, sv, false, Some(r))
        }
        0x08 => {
            let r = rx.rotate_left((ry as i32).rem_euclid(32) as u32);
            (r == 0, false, false, Some(r))
        }
        0xC0 | 0xC4 | 0xC8 => {
            let r = match (bit_ok, op) {
                (false, _) => rx,
                (true, 0xC0) => rx | (1 << bit),
                (true, 0xC4) => rx & !(1 << bit),
                (true, _) => rx ^ (1 << bit),
            };
            (r == 0, !bit_ok, false, Some(r))
        }
        0xCC => {
            let zero = !bit_ok || (rx >> bit) & 1 == 0;
            (zero, !bit_ok, false, None)
        }
        0x44 | 0x4C | 0x64 | 0x6C => {
            let (d, over) = deposit(rx, len, pos, op & 8 != 0);
            let r = d | if op & 0x20 != 0 { old } else { 0 };
            (r == 0, over, false, Some(r))
        }
        0x40 | 0x48 => {
            let (r, over) = extract(rx, len, pos, op & 8 != 0);
            (r == 0, over, false, Some(r))
        }
        0x80 | 0x84 => {
            let sign = rx >> 31 == 1;
            let leading = if sign {
                rx.leading_ones()
            } else {
                rx.leading_zeros()
            };
            let extended = op == 0x84 && v.f.av;
            let exp = if extended { 1 } else { 1 - leading as i32 };
            let ss = if op == 0x84 { v.f.av ^ sign } else { sign };
            (exp == 0, false, ss, Some(exp as u32))
        }
        0x90 => {
            let (packed, overflow) = fpack(rx);
            (false, overflow, false, Some(packed))
        }
        0x94 => (false, false, false, Some(funpack(rx))),
        // 0x88 LEFTZ and 0x8C LEFTO
        _ => {
            let count = if op == 0x88 {
                rx.leading_zeros()
            } else {
                rx.leading_ones()
            };
            let msb = rx >> 31 == 1;
            (msb == (op == 0x88), count == 32, false, Some(count))
        }
    }
}

/// The effect of shifter operation `op` writing `Rn` from the operands `rx` and `ry`.
#[inline(always)]
pub(super) fn with_operands(v: &View<'_>, op: u8, n: u8, rx: u32, ry: u32) -> Eff {
    let (sz, sv, ss, result) = unit(v, op, n, rx, ry);
    let mut e = Eff::NONE;
    if let Some(r) = result {
        e.write(n, r, false);
    }
    e.shf = Some((sz, sv, ss));
    e.astat_clear = SF;
    e
}

/// The register form: `Rn = op Rx BY Ry` of the compute word `a`.
#[inline(always)]
pub(super) fn effect(v: &View<'_>, op: u8, a: u32) -> Eff {
    let (n, x, y) = nxy(a);
    with_operands(v, op, n, v.r[usize::from(x)], v.r[usize::from(y)])
}

/// The immediate form: `a` = `op | n << 8 | x << 12`, `data` the 12-bit immediate.
#[inline(always)]
pub(super) fn immediate(v: &View<'_>, a: u32, data: u32) -> Eff {
    let (op, n, x) = (a as u8, ((a >> 8) & 15) as u8, ((a >> 12) & 15) as u8);
    with_operands(v, op, n, v.r[usize::from(x)], imm_ry(op, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::exec::h_fixed::eff::view;
    use crate::core::uop::exec::h_fixed::kinds::word;

    fn run(op: u8, rx: u32, ry: u32, old: u32) -> Eff {
        let mut c = core_with(&[0x0001]);
        (c.r[1], c.r[2], c.r[3]) = (rx, ry, old);
        effect(&view(&c, false), op, word(op, 3, 1, 2))
    }

    fn flags(e: &Eff) -> (bool, bool, bool) {
        e.shf.unwrap()
    }

    #[test]
    fn shifts_follow_the_sign_of_the_count_and_the_or_form_keeps_the_old_register() {
        assert_eq!(run(0x00, 0x8000_0001, 4, 0).wr[0].v, 0x10);
        assert_eq!(
            run(0x00, 0x8000_0000, (-4i32) as u32, 0).wr[0].v,
            0x0800_0000
        );
        assert_eq!(
            run(0x04, 0x8000_0000, (-4i32) as u32, 0).wr[0].v,
            0xF800_0000
        );
        assert_eq!(run(0x20, 1, 4, 0x100).wr[0].v, 0x110, "OR form");
        assert_eq!(
            flags(&run(0x00, 1, 40, 0)),
            (true, true, false),
            "off scale left"
        );
        assert_eq!(flags(&run(0x00, 1, 0, 0)), (false, false, false));
        // ASHIFT left sets SV only when a significant bit is lost
        assert!(!flags(&run(0x04, 1, 4, 0)).1);
        assert!(flags(&run(0x04, 0x4000_0000, 1, 0)).1);
    }

    #[test]
    fn every_operation_clears_sf_keeps_the_extension_and_btst_writes_nothing() {
        for op in SHIFT_OPS {
            let e = run(op, 0x1234_5678, 0x0000_0805, 0xFFFF_0000);
            assert_eq!(e.astat_clear, SF, "op {op:#x}");
            assert!(e.shf.is_some());
            if op == 0xCC {
                assert_eq!(e.nw, 0, "BTST has no result");
            } else {
                assert!(
                    e.nw == 1 && !e.wr[0].clear_lo && e.wr[0].n == 3,
                    "op {op:#x}"
                );
            }
            assert_eq!(e.sticky, 0, "the shifter raises nothing");
        }
    }

    #[test]
    fn rotate_bit_operations_and_counts() {
        assert_eq!(run(0x08, 0x8000_0001, 1, 0).wr[0].v, 3);
        assert_eq!(
            run(0x08, 0x8000_0001, (-1i32) as u32, 0).wr[0].v,
            0xC000_0000
        );
        assert_eq!(run(0xC0, 0, 5, 0).wr[0].v, 0x20);
        assert_eq!(run(0xC4, 0xFF, 0, 0).wr[0].v, 0xFE);
        assert_eq!(run(0xC8, 0x10, 4, 0).wr[0].v, 0);
        assert_eq!(
            flags(&run(0xC0, 7, 32, 0)),
            (false, true, false),
            "off scale"
        );
        assert!(
            flags(&run(0xCC, 0b10, 0, 0)).0,
            "BTST of a zero bit sets SZ"
        );
        assert!(!flags(&run(0xCC, 0b10, 1, 0)).0);
        assert_eq!(run(0x88, 0x0000_FFFF, 0, 0).wr[0].v, 16);
        assert_eq!(run(0x8C, 0xFFF0_0000, 0, 0).wr[0].v, 12);
        assert_eq!(
            flags(&run(0x88, 0, 0, 0)),
            (false, true, false),
            "LEFTZ of 0 is 32"
        );
        assert_eq!(run(0x80, 1, 0, 0).wr[0].v as i32, -30);
        assert!(
            flags(&run(0x80, 0xFFFF_FFFF, 0, 0)).2,
            "EXP of a negative sets SS"
        );
    }

    #[test]
    fn exp_ex_reads_the_alu_overflow_flag_of_the_element() {
        let mut c = core_with(&[0x0001]);
        (c.r[1], c.s[1]) = (0x4000_0000, 0x4000_0000);
        (c.flags.av, c.flags_y.av) = (true, false);
        let w = word(0x84, 3, 1, 2);
        assert_eq!(effect(&view(&c, false), 0x84, w).wr[0].v, 1);
        assert_eq!(
            effect(&view(&c, true), 0x84, w).wr[0].v,
            0,
            "PEy reads its own AV"
        );
    }

    #[test]
    fn field_operations_and_the_packed_float() {
        let ry = (14 << 6) | 13;
        assert_eq!(run(0x44, 0xFFFF_3FFF, ry, 0).wr[0].v, 0x3FFF << 13);
        assert_eq!(run(0x4C, 0b100, (3 << 6) | 4, 0).wr[0].v, 0xFFFF_FFC0);
        assert_eq!(
            run(0x64, 0b11, (2 << 6) | 4, 1).wr[0].v,
            0x31,
            "OR form keeps Rn"
        );
        assert_eq!(run(0x40, 0xABC, (8 << 6) | 4, 0).wr[0].v, 0xAB);
        assert_eq!(run(0x48, 0xF8C, (8 << 6) | 4, 0).wr[0].v, 0xFFFF_FFF8);
        assert_eq!(run(0x90, 1.0f32.to_bits(), 0, 0).wr[0].v, 0x3800);
        assert_eq!(f32::from_bits(run(0x94, 0x3800, 0, 0).wr[0].v), 1.0);
        assert!(
            flags(&run(0x90, 1.0e10f32.to_bits(), 0, 0)).1,
            "FPACK overflow sets SV"
        );
    }

    #[test]
    fn the_immediate_count_is_eight_bits_signed_only_for_the_shifting_operations() {
        assert_eq!(imm_ry(0x04, 0xFFF), u32::MAX, "ASHIFT BY -1");
        assert_eq!(imm_ry(0x00, 0x1_04), 4, "bits 11..8 are ignored");
        assert_eq!(imm_ry(0x00, 0x100), 0, "BY 256 shifts by 0");
        assert_eq!(imm_ry(0x44, 0xFFF), 0xFFF, "FDEP takes len6:bit6");
        assert_eq!(imm_ry(0xC8, 0x800), 0x800);
        let mut c = core_with(&[0x0001]);
        c.r[4] = 0x8000_0002;
        let a = 0x04 | 3 << 8 | 4 << 12;
        let e = immediate(&view(&c, false), a, 0xFFF);
        assert_eq!((e.wr[0].n, e.wr[0].v), (3, 0xC000_0001));
    }

    #[test]
    fn the_list_is_ascending_and_free_of_the_bit_fifo_forms() {
        assert!(SHIFT_OPS.windows(2).all(|w| w[0] < w[1]));
        for fifo in [0x50, 0x58, 0x70, 0x74, 0x7C] {
            assert!(!SHIFT_OPS.contains(&fifo));
        }
    }
}
