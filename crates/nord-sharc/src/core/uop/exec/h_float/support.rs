//! State and instruction generators shared by the float differential tests (`diff.rs`,
//! `block.rs`).
//!
//! OWNER: W2-FLOAT, test only. Everything here builds inputs: the float compute fields under test,
//! the Type 2a instruction around one, and a core with a hostile random state (design 6.2) that
//! two calls with the same seed reproduce exactly.
use crate::core::support::core_with;
use crate::core::uop::kernels::grid;
use crate::core::uop::verify::gen::{
    fill_astat_extra, fill_fifo, fill_stale_ext, fill_state, fill_sticky, Rng,
};
use crate::core::Core;

/// The float ALU opcodes with a handler.
/// VERIFIED: the opcode numbers of `Core::float_alu`; the set equals what the lowering takes
/// (`lower/float/census.rs`, `every_opcode_the_family_takes_is_run_by_the_handler_tests`) and every
/// one is a float op of the family (`every_generated_compute_is_a_float_op_the_lowering_takes`).
pub(in crate::core) const ALU_OPS: [u8; 21] = [
    0x81, 0x82, 0x89, 0x8A, 0xA1, 0xA2, 0xB0, 0xE0, 0xE1, 0xE2, 0xE3, 0xA5, 0xBD, 0xC9, 0xD9, 0xCD,
    0xDD, 0xCA, 0xDA, 0xC4, 0xC5,
];
/// MODE1 bits SRRFL (R0-R7 and S0-S7 secondary) and SRRFH (R8-R15, S8-S15).
/// VERIFIED: `banks.rs` (`FILE_SETS`), checked by `banks::tests`.
pub(super) const SRRFL: u32 = 1 << 10;
pub(super) const SRRFH: u32 = 1 << 7;

/// `IF cond <compute>` as one Type 2a instruction (layout of `[0x013e, 0, 0x1312]`).
pub(super) fn t2a(cond: u16, compute: u32) -> [u16; 3] {
    [
        0x0100 | cond << 1,
        (compute >> 16) as u16 & 0x7F,
        compute as u16,
    ]
}

/// A float ALU compute field.
pub(super) fn alu(op: u8, n: u8, x: u8, y: u8) -> u32 {
    u32::from(op) << 12 | u32::from(n) << 8 | u32::from(x) << 4 | u32::from(y)
}

/// A float multiplier compute field (unit 1, opcode 0x30).
pub(super) fn mul(n: u8, x: u8, y: u8) -> u32 {
    1 << 20 | 0x30 << 12 | u32::from(n) << 8 | u32::from(x) << 4 | u32::from(y)
}

/// A float multiplier + float ALU multifunction: bit 22, unit bits 21..19 = 0b011, the ALU op in
/// bits 18..16 and the six register fields below.
pub(super) fn multi(rng: &mut Rng) -> u32 {
    1 << 22 | 0b011 << 19 | rng.below(8) << 16 | rng.below(1 << 16)
}

/// Register numbers that collide often: aliasing is exercised as much as distinct registers.
fn reg(rng: &mut Rng) -> u8 {
    if rng.below(2) == 0 {
        rng.below(16) as u8
    } else {
        rng.below(3) as u8
    }
}

/// Every float compute field under test with random registers.
pub(super) fn computes(rng: &mut Rng) -> Vec<u32> {
    let mut v: Vec<u32> = ALU_OPS
        .iter()
        .map(|&op| alu(op, reg(rng), reg(rng), reg(rng)))
        .collect();
    v.push(mul(reg(rng), reg(rng), reg(rng)));
    v.push(multi(rng));
    v
}

/// One register: a grid float with a live extension, or a stale one (the byte of another value).
fn register(rng: &mut Rng) -> (u32, u32, u8) {
    let v = grid::operand(rng);
    let r = (v >> 8) as u32;
    if rng.below(3) != 0 {
        (r, r, v as u8)
    } else {
        (r, r.wrapping_add(1 + rng.below(5)), rng.next_u32() as u8)
    }
}

/// A core with `parcels` at the pc and the state drawn from `seed`: the same call twice gives two
/// identical cores. `toggle` is the MODE1 mask of SRRFL and SRRFH bits to flip afterwards.
pub(super) fn prepare(parcels: &[u16], seed: u64, simd: bool, toggle: u32) -> Core {
    let mut c = core_with(parcels);
    let mut rng = Rng::new(seed);
    fill_state(&mut c, &mut rng, simd);
    for k in 0..16 {
        (c.r[k], c.fx.tag[k], c.fx.lo[k]) = register(&mut rng);
        (c.s[k], c.fx_y.tag[k], c.fx_y.lo[k]) = register(&mut rng);
    }
    let b = rng.next_u32();
    (c.flags.ai, c.flags.as_, c.flags.sv, c.flags.sz, c.flags.tf) =
        (b & 1 != 0, b & 2 != 0, b & 4 != 0, b & 8 != 0, b & 16 != 0);
    (
        c.flags_y.ai,
        c.flags_y.as_,
        c.flags_y.sv,
        c.flags_y.sz,
        c.flags_y.tf,
    ) = (
        b & 32 != 0,
        b & 64 != 0,
        b & 128 != 0,
        b & 256 != 0,
        b & 512 != 0,
    );
    fill_sticky(&mut c, &mut rng);
    fill_astat_extra(&mut c, &mut rng);
    fill_fifo(&mut c, &mut rng);
    if rng.below(2) == 0 {
        fill_stale_ext(&mut c, &mut rng);
    }
    if toggle != 0 {
        c.write_ureg(0x72, c.mode1 ^ toggle);
    }
    c.irq.due = u64::MAX;
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::decode_cache::ComputeOp;
    use crate::core::uop::lower::float::select;
    use crate::core::uop::verify::compare::exact_diff;

    #[test]
    fn every_generated_compute_is_a_float_op_the_lowering_takes() {
        let mut rng = Rng::new(7);
        for _ in 0..50 {
            let all = computes(&mut rng);
            assert_eq!(all.len(), ALU_OPS.len() + 2);
            for c in all {
                let op = ComputeOp::decode(u64::from(c));
                assert!(
                    select(op).is_some(),
                    "{c:#x} is not a float op of the family"
                );
            }
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_core_and_a_toggle_flips_only_the_bank_bits() {
        let p = t2a(31, alu(0x81, 1, 2, 3));
        let (a, b) = (prepare(&p, 11, true, 0), prepare(&p, 11, true, 0));
        assert_eq!(exact_diff(&a, &b), None);
        assert_eq!(a.irq.due, u64::MAX);
        assert_ne!(
            exact_diff(&a, &prepare(&p, 12, true, 0)),
            None,
            "another seed"
        );
        let t = prepare(&p, 11, true, SRRFL | SRRFH);
        assert_eq!(t.mode1 ^ a.mode1, SRRFL | SRRFH);
    }

    #[test]
    fn the_states_hold_stale_and_live_extensions_and_a_toggle_is_seen_by_the_comparison() {
        let p = t2a(31, alu(0x81, 1, 2, 3));
        // registers whose extension belongs to an older value (reads as 0) among all of them
        let stale = |c: &Core| {
            (0..16)
                .filter(|&n| c.fx.tag[n] != c.r[n] && c.fx.lo[n] != 0)
                .count()
        };
        let n: usize = (0..60u64).map(|s| stale(&prepare(&p, s, false, 0))).sum();
        assert!(
            n > 60 * 16 / 10 && n < 60 * 16 * 9 / 10,
            "{n} of {} registers have a stale extension",
            60 * 16
        );
        let (a, b) = (prepare(&p, 3, false, 0), prepare(&p, 3, false, SRRFL));
        assert_ne!(
            exact_diff(&a, &b),
            None,
            "the toggle is visible to the comparison"
        );
    }
}
