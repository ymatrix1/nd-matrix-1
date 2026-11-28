//! Inputs of the fixed-point differential tests: instruction encoders, the full-state filler and
//! the program generators.
//!
//! OWNER: W2-FIXED, test support. The filler writes everything a fixed-point compute can read or
//! write (both elements' registers and float extensions, MR, flags, ASTAT extra bits, STKY,
//! MODE1) from one seed, so a case is reproducible and three cores built from it are identical.
use super::kinds::word;
use crate::core::float::MODE1_ALUSAT;
use crate::core::support::core_with;
use crate::core::types::Flags;
use crate::core::uop::verify::gen::{fill_astat_extra, fill_sticky, Rng};
use crate::core::{Core, MODE1_PEYEN};

/// `IF cond <compute c>` (T2a): condition code in bits 5..1 of the first parcel, the 23-bit
/// compute field in bits 22..0 of the word. VERIFIED: `DecodedOp::from_insn` and the encodings of
/// `decode_cache.rs` tests, checked by `the_encoders_decode_as_intended`.
pub(super) fn t2a(cond: u8, c: u32) -> Vec<u16> {
    vec![0x0100 | u16::from(cond) << 1, (c >> 16) as u16, c as u16]
}

/// Unconditional compute (T2b, 32 bits): bits 38..16 of the word are the compute field.
pub(super) fn t2b(c: u32) -> Vec<u16> {
    vec![0x0180 | (c >> 16) as u16, c as u16]
}

/// `IF cond Rn = <shifter op> Rx BY <imm12>` (T6n): op6 in bits 21..16, the immediate in bits
/// 30..27 (high nibble) and 15..8 (low byte), `n` in bits 7..4 and `x` in bits 3..0.
pub(super) fn t6n(cond: u8, op6: u8, n: u8, x: u8, data: u32) -> Vec<u16> {
    vec![
        0x0200 | u16::from(cond) << 1,
        ((data >> 8 & 15) << 11) as u16 | u16::from(op6),
        ((data & 0xFF) << 8) as u16 | u16::from(n) << 4 | u16::from(x),
    ]
}

/// A 32-bit operand: a boundary of the fixed-point rules, a small count or position, a field
/// descriptor, a value of random magnitude or random bits. ASSUMED: the mix is a plain one; the
/// per-kind minimums of the tests show every outcome class is reached.
pub(super) fn fixed_word(rng: &mut Rng) -> u32 {
    const EDGES: [u32; 14] = [
        0,
        1,
        u32::MAX,
        0x8000_0000,
        0x7FFF_FFFF,
        0x8000_0001,
        0x7FFF_FFFE,
        0xFFFF,
        0x1_0000,
        0x4000_0000,
        0xC000_0000,
        0x2000_0000,
        0x8000,
        0xFFFF_8000,
    ];
    match rng.below(10) {
        0..=2 => rng.pick(&EDGES),
        3 | 4 => (rng.below(80) as i32 - 40) as u32,
        5 => rng.next_u32() & 0xFFF,
        6 => rng.next_u32() >> rng.below(32),
        7 => (rng.next_u32() >> rng.below(32)).wrapping_neg(),
        _ => rng.next_u32(),
    }
}

/// An 80-bit multiplier result register value (sign extended in the `i128`): boundaries of the
/// 32, 64 and 80 bit windows, rounding ties, values with MR1 all zeros or all ones, fractional
/// and integer looking values, full 80 bit values.
pub(super) fn mr_word(rng: &mut Rng) -> i128 {
    const EDGES: [i128; 20] = [
        0,
        1,
        -1,
        0x7FFF_FFFF,
        0x8000_0000,
        0xFFFF_FFFF,
        1 << 32,
        (1 << 32) - 1,
        -(1 << 32),
        1 << 63,
        (1 << 63) - 1,
        -(1 << 63),
        (1 << 64) - 1,
        1 << 64,
        (1 << 79) - 1,
        -(1 << 79),
        0x1_8000_0000,
        0x2_8000_0000,
        -0x1_8000_0000,
        (1 << 63) - 0x8000_0000,
    ];
    let wrap = |v: i128| (v << 48) >> 48;
    match rng.below(9) {
        0..=2 => rng.pick(&EDGES),
        3 => i128::from(rng.next_u64() as i64),
        4 => wrap(i128::from(rng.next_u64()) << 16 | i128::from(rng.next_u32() & 0xFFFF)),
        5 => i128::from(rng.next_u32()),
        6 => (i128::from(rng.below(3)) - 1) << 32 | i128::from(rng.next_u32()),
        // a rounding tie (bit 31 only below) under 48 random bits: 80 bits in all
        7 => i128::from((rng.next_u64() as i64) >> 16) << 32 | 0x8000_0000,
        _ => wrap(i128::from(rng.next_u64() as i64) << rng.below(17)),
    }
}

/// A register number, the first four more often than the rest so operands alias often.
pub(super) fn reg(rng: &mut Rng) -> u32 {
    if rng.below(2) == 0 {
        rng.below(4)
    } else {
        rng.below(16)
    }
}

/// The float extension of a register: live, stale, absent or a tag-only leftover.
fn ext(rng: &mut Rng, r: u32) -> (u32, u8) {
    let lo = 1 + rng.below(255) as u8;
    match rng.below(4) {
        0 => (r, lo),
        1 => (r.wrapping_add(1 + rng.below(5)), lo),
        2 => (0, 0),
        _ => (r, 0),
    }
}

fn random_flags(rng: &mut Rng) -> Flags {
    let b = rng.next_u32();
    Flags {
        az: b & 1 != 0,
        an: b & 2 != 0,
        av: b & 4 != 0,
        ac: b & 8 != 0,
        ai: b & 16 != 0,
        as_: b & 32 != 0,
        tf: b & 64 != 0,
        sz: b & 128 != 0,
        sv: b & 256 != 0,
        ss: b & 512 != 0,
        ..Flags::default()
    }
}

/// Fills everything a fixed-point compute can read or write: the data registers of both
/// elements with their float extensions, the multiplier result registers, the ALU and shifter
/// flags, the ASTAT extra bits (MN..MI, AF, SF, CACC), STKY and MODE1 (ALUSAT, PEYEN). The
/// interrupt controller is quiet (`irq.due = u64::MAX`) so the step has no prologue.
pub(super) fn fixed_state(c: &mut Core, rng: &mut Rng, simd: bool) {
    for k in 0..16 {
        c.r[k] = fixed_word(rng);
        c.s[k] = fixed_word(rng);
        (c.fx.tag[k], c.fx.lo[k]) = ext(rng, c.r[k]);
        (c.fx_y.tag[k], c.fx_y.lo[k]) = ext(rng, c.s[k]);
    }
    c.mr = [mr_word(rng), mr_word(rng)];
    c.mr_y = [mr_word(rng), mr_word(rng)];
    (c.flags, c.flags_y) = (random_flags(rng), random_flags(rng));
    let modes = if rng.below(2) == 0 { MODE1_ALUSAT } else { 0 };
    c.mode1 =
        (c.mode1 & !(MODE1_ALUSAT | MODE1_PEYEN)) | modes | if simd { MODE1_PEYEN } else { 0 };
    fill_astat_extra(c, rng);
    fill_sticky(c, rng);
    c.irq.due = u64::MAX;
}

/// The same core every time for `(program, seed, simd)`.
pub(super) fn core_for(program: &[u16], seed: u64, simd: bool) -> Core {
    let mut c = core_with(program);
    fixed_state(&mut c, &mut Rng::new(seed), simd);
    c
}

/// `count` random programs per opcode of compute unit `unit` (0 ALU, 1 multiplier, 2 shifter):
/// random operand registers (low registers often, so operands alias), and one of the three forms
/// of a conditional or unconditional compute: `IF TRUE`, `IF <random condition>`, or no condition
/// (T2b). The first program of each opcode is `IF TRUE`, so that every opcode runs on every
/// state (some condition codes are false in every generated state: pins, loop counters).
/// ASSUMED: the mix; the tests count what ran per kind.
pub(super) fn random_programs(unit: u32, ops: &[u8], count: usize, rng: &mut Rng) -> Vec<Vec<u16>> {
    let mut v = Vec::with_capacity(ops.len() * count);
    for &op in ops {
        for k in 0..count {
            let c = unit << 20 | word(op, reg(rng) as u8, reg(rng) as u8, reg(rng) as u8);
            v.push(match rng.below(6) {
                _ if k == 0 => t2a(31, c),
                0 => t2b(c),
                1 | 2 => t2a(31, c),
                _ => t2a(rng.below(31) as u8, c),
            });
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::decode_cache::{ComputeOp, DecodedOp};
    use crate::core::uop::exec::h_fixed::kinds::imm_b;
    use crate::core::uop::lower::decode::decode_at;
    use crate::core::uop::verify::compare::exact_diff;

    #[test]
    fn random_programs_are_whole_computes_of_the_unit_asked_for() {
        let mut rng = Rng::new(5);
        let progs = random_programs(2, &[0x04, 0x08], 40, &mut rng);
        assert_eq!(progs.len(), 80);
        let (mut conds, mut forms) = (std::collections::BTreeSet::new(), [0usize; 2]);
        for p in &progs {
            let mut c = core_with(p);
            let pc = c.pc;
            match decode_at(&mut c, pc).op {
                DecodedOp::ComputeCond {
                    cond,
                    op: ComputeOp::Shifter { op, .. },
                } => {
                    assert!(op == 0x04 || op == 0x08);
                    conds.insert(cond);
                    forms[0] += 1;
                }
                DecodedOp::ComputeBoth {
                    op: ComputeOp::Shifter { .. },
                } => forms[1] += 1,
                other => panic!("{p:04x?}: {other:?}"),
            }
        }
        assert!(
            conds.len() > 8 && conds.contains(&31),
            "conditions: {conds:?}"
        );
        assert!(forms[0] > 40 && forms[1] > 5, "forms {forms:?}");
    }

    #[test]
    fn the_encoders_decode_as_intended() {
        let mut c = core_with(&t2a(5, 0x01_312));
        let pc = c.pc;
        let d = decode_at(&mut c, pc);
        assert_eq!(
            d.op,
            DecodedOp::ComputeCond {
                cond: 5,
                op: ComputeOp::FixedAlu {
                    op: 1,
                    n: 3,
                    x: 1,
                    y: 2
                }
            }
        );
        let mut c = core_with(&t2b(0x02_312));
        let d = decode_at(&mut c, pc);
        assert!(matches!(
            d.op,
            DecodedOp::ComputeBoth {
                op: ComputeOp::FixedAlu { op: 2, .. }
            }
        ));
        let mut c = core_with(&t6n(31, 1, 1, 1, 0xFFF));
        let d = decode_at(&mut c, pc);
        assert_eq!(
            (d.op, d.cond, d.w & 0xFFFF_FFFF_FFFF),
            (DecodedOp::ShiftImm, 31, 0x023e_7801_ff11)
        );
        assert_eq!(super::super::kinds::imm_of(imm_b(31, 0xFFF)), 0xFFF);
    }

    #[test]
    fn the_state_is_reproducible_and_covers_what_an_op_reads() {
        let (a, b) = (core_for(&[0x0001], 7, true), core_for(&[0x0001], 7, true));
        assert!(
            exact_diff(&a, &b).is_none(),
            "the same seed gives the same core"
        );
        assert!(exact_diff(&a, &core_for(&[0x0001], 8, true)).is_some());
        assert!(
            a.mode1 & MODE1_PEYEN != 0 && core_for(&[0x0001], 7, false).mode1 & MODE1_PEYEN == 0
        );
        let mut seen = (false, false, false, false);
        let mut rng = Rng::new(3);
        for _ in 0..200 {
            let m = mr_word(&mut rng);
            assert_eq!(m, (m << 48) >> 48, "an 80 bit value");
            seen.0 |= m < -(1 << 63);
            seen.1 |= m >= 1 << 63;
            seen.2 |= (1..=0xFFFF_FFFF).contains(&m);
            seen.3 |= m & 0xFFFF_FFFF == 0x8000_0000;
        }
        assert_eq!(
            seen,
            (true, true, true, true),
            "spill, window and tie values all occur"
        );
        assert!(a.fx.lo.iter().any(|&l| l != 0) && a.fx_y.tag.iter().any(|&t| t != 0));
    }
}
