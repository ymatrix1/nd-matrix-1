//! OWNER: W2-NDP (tests). The generators and sweeps of the native data-processing differential
//! suite: uops built by hand from the field table of the design note (3.1), random shapes and
//! states, and one sweep function per family that runs native against tier 2 on every case and
//! returns the oracle's [`Tally`]. `diff.rs` runs them at full size and asserts they exercised
//! something; `kill.rs` reruns reduced forms under each mutant.
//!
//! Test inputs are ASSUMED (no hardware claim): edge values at the carry, overflow and shifter
//! boundaries of 32-bit arithmetic and the register-shift cut-offs 0, 32, 33, 64, 256; the
//! oracle is `exec_alu::exec_dp` through `exec::run_sb`, itself VERIFIED against
//! `Cpu::dp_exec` (`exec_alu/diff.rs`).

use super::harness::{random_state, Rig, Tally, PC};
use crate::cpu::Cpu;
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::uop::{k, unpack_flags, Uop};

/// Edge register values: zero, one, the sign boundary, all ones, alternating bits, byte limits.
#[rustfmt::skip]
pub(super) const VALS: [u32; 13] = [
    0, 1, 2, 0x7FFF_FFFE, 0x7FFF_FFFF, 0x8000_0000, 0x8000_0001, 0xFFFF_FFFE, 0xFFFF_FFFF,
    0x5555_5555, 0xAAAA_AAAA, 0xFF, 0x100,
];

/// Register-shift amounts (low byte), with noise in the upper bits added by the sweeps.
pub(super) const AMOUNTS: [u32; 17] = [
    0, 1, 2, 30, 31, 32, 33, 63, 64, 65, 127, 128, 255, 256, 257, 300, 511,
];

/// Operand fields of one generated uop: `regs` is `[rd, rn, rm, rs]`.
#[derive(Clone, Copy, Default)]
pub(super) struct Shape {
    pub regs: [u8; 4],
    pub imm8: u32,
    pub rot: u32,
    pub ty: u32,
    pub amount: u32,
}

/// The generic uop of `kind` (`0..=95`) with the operand fields of `p`, built from the field
/// table of the design note (3.1).
pub(super) fn generic_uop(kind: u8, p: Shape) -> Uop {
    let [rd, rn, rm, rs] = p.regs;
    let mut u = Uop {
        rd,
        rn,
        rm,
        rs,
        ..Uop::new(kind, PC)
    };
    match kind >> 5 {
        0 => {
            u.imm = p.imm8.rotate_right(2 * p.rot);
            u.sh = (p.rot == 0) as u8;
        }
        1 => u.sh = (p.ty << 5 | p.amount) as u8,
        _ => u.sh = (p.ty << 5) as u8,
    }
    u
}

/// A random shape: registers `0..=14` with the aliasings that matter (`rd == rn`, `rm == rd`,
/// `rm == rn`, `rs == rm`, ...), immediates and amounts biased to their edges.
pub(super) fn random_shape(rng: &mut Rng) -> Shape {
    let mut regs = [0u8; 4];
    for v in &mut regs {
        *v = rng.below(15) as u8;
    }
    match rng.below(8) {
        0 => regs[1] = regs[0],
        1 => regs[2] = regs[0],
        2 => regs[2] = regs[1],
        3 => regs[3] = regs[2],
        4 => regs[3] = regs[0],
        5 => (regs[1], regs[2]) = (regs[0], regs[0]),
        _ => {}
    }
    Shape {
        regs,
        imm8: if rng.chance(2) {
            rng.pick(&[0, 1, 0x80, 0xFF])
        } else {
            rng.below(256)
        },
        rot: if rng.chance(4) { 0 } else { rng.below(16) },
        ty: rng.below(4),
        amount: if rng.chance(2) {
            rng.pick(&[0, 1, 2, 15, 30, 31])
        } else {
            rng.below(32)
        },
    }
}

/// A random state in which the shift-amount register of form 2 holds an edge amount.
fn state_for(rng: &mut Rng, u: &Uop) -> Cpu {
    let mut cpu = random_state(rng);
    if u.kind < 96 && u.kind >> 5 == 2 && !rng.chance(4) {
        let low = if rng.flag() {
            AMOUNTS[rng.below(AMOUNTS.len() as u32) as usize] & 0xFF
        } else {
            rng.below(256)
        };
        cpu.r[u.rs as usize] = rng.next() & 0xFFFF_FF00 | low;
    }
    cpu
}

/// `shapes` random shapes x `states` random states for every generic id in `kinds`; one oracle
/// tally per id.
pub(super) fn sweep_generic(
    rig: &mut Rig,
    rng: &mut Rng,
    kinds: std::ops::Range<u8>,
    shapes: u32,
    states: u32,
) -> Result<Vec<(u8, Tally)>, String> {
    let mut all = Vec::new();
    for kind in kinds {
        let mut t = Tally::default();
        for _ in 0..shapes {
            let u = generic_uop(kind, random_shape(rng));
            let sb = Rig::trace(&[u]);
            let native = rig.compile(&sb);
            for _ in 0..states {
                let cpu = state_for(rng, &u);
                rig.check(&sb, &native, &cpu, &mut t)
                    .map_err(|e| format!("generic kind {kind}: {e}"))?;
            }
        }
        all.push((kind, t));
    }
    Ok(all)
}

/// The 12 specialised ids on random shapes and states.
pub(super) fn sweep_specials(
    rig: &mut Rig,
    rng: &mut Rng,
    shapes: u32,
    states: u32,
) -> Result<Vec<(u8, Tally)>, String> {
    let mut all = Vec::new();
    for kind in k::MOV_IMM..=k::ORR_IMM {
        let mut t = Tally::default();
        for _ in 0..shapes {
            let p = random_shape(rng);
            let mut u = Uop::new(kind, PC);
            (u.rd, u.rn, u.rm) = (p.regs[0], p.regs[1], p.regs[2]);
            u.imm = if rng.chance(3) {
                rng.pick(&VALS)
            } else {
                p.imm8.rotate_right(2 * p.rot)
            };
            u.sh = u8::from(kind == k::TST_IMM && rng.flag());
            let sb = Rig::trace(&[u]);
            let native = rig.compile(&sb);
            for _ in 0..states {
                let cpu = random_state(rng);
                rig.check(&sb, &native, &cpu, &mut t)
                    .map_err(|e| format!("special kind {kind}: {e}"))?;
            }
        }
        all.push((kind, t));
    }
    Ok(all)
}

/// A state with `r1 = a`, `r2 = b` (the rest random) and flag nibble `f`.
fn state_ab(rng: &mut Rng, a: u32, b: u32, f: u8) -> Cpu {
    let mut cpu = random_state(rng);
    cpu.r[1] = a;
    cpu.r[2] = b;
    unpack_flags(&mut cpu, f);
    cpu
}

/// Every opcode in `ops` (S and not S) with `rd = 3, rn = 1, rm = 2` and the three aliasings,
/// on every pair of edge values x 16 flag nibbles.
pub(super) fn sweep_arith_edges(
    rig: &mut Rig,
    rng: &mut Rng,
    ops: &[u32],
    vals: &[u32],
) -> Result<Tally, String> {
    let mut t = Tally::default();
    for &op in ops {
        for s in [false, true] {
            for regs in [[3u8, 1, 2, 4], [1, 1, 2, 4], [2, 1, 2, 4], [1, 1, 1, 4]] {
                let u = generic_uop(
                    k::dp(1, op, s),
                    Shape {
                        regs,
                        ..Shape::default()
                    },
                );
                let sb = Rig::trace(&[u]);
                let native = rig.compile(&sb);
                for &a in vals {
                    for &b in vals {
                        for f in 0..16u8 {
                            let cpu = state_ab(rng, a, b, f);
                            rig.check(&sb, &native, &cpu, &mut t)
                                .map_err(|e| format!("op {op} s {s} regs {regs:?}: {e}"))?;
                        }
                    }
                }
            }
        }
    }
    Ok(t)
}

/// Form 1 `movs r3, r1, <ty> #amount` for the given types and amounts on every edge value x 16
/// flag nibbles: the value and the carry of every static shift, `lsr #0`, `asr #0` and RRX.
pub(super) fn sweep_shift_imm(
    rig: &mut Rig,
    rng: &mut Rng,
    tys: &[u32],
    amounts: &[u32],
) -> Result<Tally, String> {
    let mut t = Tally::default();
    for &ty in tys {
        for &amount in amounts {
            for op in [13u32, 15] {
                let regs = [3, 1, 1, 4];
                let u = generic_uop(
                    k::dp(1, op, true),
                    Shape {
                        regs,
                        ty,
                        amount,
                        ..Shape::default()
                    },
                );
                let sb = Rig::trace(&[u]);
                let native = rig.compile(&sb);
                for &a in &VALS {
                    for f in 0..16u8 {
                        let cpu = state_ab(rng, a, 0, f);
                        rig.check(&sb, &native, &cpu, &mut t)
                            .map_err(|e| format!("op {op} ty {ty} #{amount}: {e}"))?;
                    }
                }
            }
        }
    }
    Ok(t)
}

/// Form 2 `movs r3, r1, <ty> r2` (and the alias `rs == rm`) for every amount in `amounts`
/// (plus noise in bits 8..32 of the register) on every edge value x 16 flag nibbles.
pub(super) fn sweep_shift_reg(
    rig: &mut Rig,
    rng: &mut Rng,
    tys: &[u32],
    amounts: &[u32],
) -> Result<Tally, String> {
    let mut t = Tally::default();
    for &ty in tys {
        for alias in [false, true] {
            let regs = if alias { [3, 1, 1, 1] } else { [3, 0, 1, 2] };
            let u = generic_uop(
                k::dp(2, 13, true),
                Shape {
                    regs,
                    ty,
                    ..Shape::default()
                },
            );
            let sb = Rig::trace(&[u]);
            let native = rig.compile(&sb);
            for &amount in amounts {
                for &a in &VALS {
                    for f in 0..16u8 {
                        let noise = if f & 1 == 0 {
                            0
                        } else {
                            rng.next() & 0xFFFF_FF00
                        };
                        let cpu = state_ab(rng, a, amount | noise, f);
                        rig.check(&sb, &native, &cpu, &mut t).map_err(|e| {
                            format!("reg shift ty {ty} amount {amount:#x} alias {alias}: {e}")
                        })?;
                    }
                }
            }
        }
    }
    Ok(t)
}

/// Rotated immediates: the rotations `rots` of the edge bytes through the opcodes `ops`.
pub(super) fn sweep_immediates(
    rig: &mut Rig,
    rng: &mut Rng,
    rots: &[u32],
    ops: &[u32],
) -> Result<Tally, String> {
    let mut t = Tally::default();
    for &rot in rots {
        for &op in ops {
            for imm8 in [0u32, 1, 0x80, 0xFF] {
                let p = Shape {
                    regs: [3, 1, 2, 4],
                    imm8,
                    rot,
                    ..Shape::default()
                };
                let sb = Rig::trace(&[generic_uop(k::dp(0, op, true), p)]);
                let native = rig.compile(&sb);
                for _ in 0..8 {
                    let cpu = random_state(rng);
                    rig.check(&sb, &native, &cpu, &mut t)
                        .map_err(|e| format!("imm op {op} rot {rot} imm8 {imm8:#x}: {e}"))?;
                }
            }
        }
    }
    Ok(t)
}

/// A `PRED` on every condition around two flag-setting uops, followed by an `adc` that reads
/// the flags the body (or its skip) left, on 16 flag nibbles x random states.
pub(super) fn sweep_pred(rig: &mut Rig, rng: &mut Rng, conds: &[u8]) -> Result<Tally, String> {
    let mut t = Tally::default();
    let (sub, mov) = (k::dp(1, 2, true), k::dp(2, 13, true));
    for &cond in conds {
        let mut pred = Uop::new(k::PRED, PC);
        (pred.cond, pred.imm, pred.n) = (cond, 2, 0);
        let body = [
            generic_uop(
                sub,
                Shape {
                    regs: [3, 1, 2, 4],
                    ..Shape::default()
                },
            ),
            generic_uop(
                mov,
                Shape {
                    regs: [5, 3, 3, 2],
                    ty: 1,
                    ..Shape::default()
                },
            ),
        ];
        let tail = generic_uop(
            k::dp(1, 5, true),
            Shape {
                regs: [6, 5, 3, 4],
                ..Shape::default()
            },
        );
        let sb = Rig::trace(&[pred, body[0], body[1], tail]);
        let native = rig.compile(&sb);
        for f in 0..16u8 {
            for _ in 0..6 {
                let mut cpu = random_state(rng);
                unpack_flags(&mut cpu, f);
                rig.check(&sb, &native, &cpu, &mut t)
                    .map_err(|e| format!("pred cond {cond} f {f:04b}: {e}"))?;
            }
        }
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_uops_follow_the_field_table_and_random_shapes_stay_below_r15() {
        let p = Shape {
            regs: [3, 1, 2, 4],
            imm8: 0xFF,
            rot: 4,
            ty: 2,
            amount: 7,
        };
        let (imm, reg, by_reg) = (
            generic_uop(k::dp(0, 13, true), p),
            generic_uop(k::dp(1, 13, true), p),
            generic_uop(k::dp(2, 13, true), p),
        );
        assert_eq!((imm.imm, imm.sh), (0xFFu32.rotate_right(8), 0));
        assert_eq!(generic_uop(k::dp(0, 13, true), Shape { rot: 0, ..p }).sh, 1);
        assert_eq!((reg.sh, by_reg.sh), (2 << 5 | 7, 2 << 5));
        assert_eq!((reg.rd, reg.rn, reg.rm, reg.rs), (3, 1, 2, 4));
        let mut rng = Rng::new(5);
        for _ in 0..500 {
            let s = random_shape(&mut rng);
            assert!(s.regs.iter().all(|r| *r < 15) && s.rot < 16 && s.amount < 32 && s.ty < 4);
        }
    }
}
