//! Differential suite of the data-processing arms against the interpreter (WP W1-XALU, layer L0
//! for this family): every `(OP, S, FORM)` arm and every specialised id against `Cpu::dp_exec`
//! on random states, edge-value sweeps of the shifter and of the arithmetic, the aliasing of
//! `rd` with the sources, and a comparator self-check plus the mutation kill test, so a run that
//! compared nothing cannot pass. A uop is built by hand from the field table of the design note
//! (3.1): the rotated immediate and the `sh` bit 0 of form 0, `type << 5 | amount` of form 1,
//! `type << 5` of form 2.

use super::*;
use crate::cpu::Cpu;
use crate::machine::sb::testkit::{fuzz_cases, random_cpu, Rng};
use crate::machine::sb::uop::{pack_flags, unpack_flags};

/// Edge register values: zero, one, the sign boundary, all ones, alternating bits, byte limits.
/// ASSUMED (test input, no hardware claim): chosen at the carry, overflow and shifter boundaries
/// of 32-bit arithmetic; the oracle is `Cpu::dp_exec`.
#[rustfmt::skip]
const VALS: [u32; 13] = [
    0, 1, 2, 0x7FFF_FFFE, 0x7FFF_FFFF, 0x8000_0000, 0x8000_0001, 0xFFFF_FFFE, 0xFFFF_FFFF,
    0x5555_5555, 0xAAAA_AAAA, 0xFF, 0x100,
];

/// Register-shift amount registers: the low byte decides, the high bits are noise.
/// ASSUMED (test input): the amounts 0, 1, 2, 30..33, 63..65, 127, 128, 255..257 straddle the
/// 5-bit, 6-bit and 8-bit cut-offs of the shifter rules; the oracle is `Cpu::dp_exec`.
#[rustfmt::skip]
const AMOUNTS: [u32; 17] = [
    0, 1, 2, 30, 31, 32, 33, 63, 64, 65, 127, 128, 255, 256, 257, 0xFFFF_FF20, 0xABCD_0000,
];

/// Parameters of one case; `regs` is `[rd, rn, rm, rs]`.
#[derive(Clone, Copy, Default)]
pub(super) struct P {
    regs: [u8; 4],
    imm8: u32,
    rot: u32,
    ty: u32,
    amount: u32,
}

/// The uop and the instruction word it stands for.
#[derive(Clone, Copy, Debug)]
pub(super) struct Case {
    pub(super) u: Uop,
    insn: u32,
    form: u32,
    op: u32,
    s: bool,
}

/// Build the case of `(form, op, s)` from the field table.
pub(super) fn build(form: u32, op: u32, s: bool, p: P) -> Case {
    let [rd, rn, rm, rs] = p.regs;
    let head = 0xE000_0000 | op << 21 | (s as u32) << 20 | (rn as u32) << 16 | (rd as u32) << 12;
    let mut u = Uop {
        rd,
        rn,
        rm,
        rs,
        ..Uop::new(k::dp(form, op, s), 0x1000)
    };
    let insn = match form {
        0 => {
            u.imm = p.imm8.rotate_right(2 * p.rot);
            u.sh = (p.rot == 0) as u8;
            head | 1 << 25 | p.rot << 8 | p.imm8
        }
        1 => {
            u.sh = (p.ty << 5 | p.amount) as u8;
            head | p.amount << 7 | p.ty << 5 | rm as u32
        }
        _ => {
            u.sh = (p.ty << 5) as u8;
            head | (rs as u32) << 8 | p.ty << 5 | 1 << 4 | rm as u32
        }
    };
    Case {
        u,
        insn,
        form,
        op,
        s,
    }
}

/// What a run proved it did.
#[derive(Default, Debug)]
pub(super) struct Tally {
    pub(super) cases: u32,
    pub(super) reg_changed: u32,
    pub(super) flags_changed: u32,
    /// Flag bits that differed from their input in at least one case.
    pub(super) bits_changed: u8,
    /// Flag bits seen set / clear in a result.
    pub(super) seen_set: u8,
    pub(super) seen_clear: u8,
}

/// A CPU with `regs` set, flags `f` and `r15` at the pc of the uop.
pub(super) fn cpu_with(regs: &[(usize, u32)], f: u8) -> Cpu {
    let mut cpu = Cpu::new();
    for &(i, v) in regs {
        cpu.r[i] = v;
    }
    cpu.r[15] = 0x1008;
    unpack_flags(&mut cpu, f);
    cpu
}

/// Run one case against `Cpu::dp_exec`; `Err` names the first difference.
pub(super) fn check(cpu: &Cpu, c: &Case, t: &mut Tally) -> Result<(), String> {
    let mut want = cpu.clone();
    want.dp_exec(c.op, c.s, c.form, c.insn);
    let (mut r, mut f) = (cpu.r, pack_flags(cpu));
    exec_dp(&mut r, &mut f, &c.u);
    let wf = want.nzcv() as u8;
    if r != want.r || f != wf {
        return Err(format!(
            "{:?}: got r {r:x?} f {f:04b}, want r {:x?} f {wf:04b} (from r {:x?} f {:04b})",
            c.u,
            want.r,
            cpu.r,
            pack_flags(cpu)
        ));
    }
    let before = pack_flags(cpu);
    t.cases += 1;
    t.reg_changed += (want.r != cpu.r) as u32;
    t.flags_changed += (wf != before) as u32;
    t.bits_changed |= wf ^ before;
    t.seen_set |= wf;
    t.seen_clear |= !wf & 0xF;
    Ok(())
}

/// A random case of `(form, op, s)` on a random state; `plain` means no shift (`lsl #0`).
pub(super) fn random_case(rng: &mut Rng, form: u32, op: u32, s: bool, plain: bool) -> (Cpu, Case) {
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
    let imm8 = if rng.chance(2) {
        rng.pick(&[0, 1, 0x80, 0xFF])
    } else {
        rng.below(256)
    };
    let rot = if rng.chance(4) { 0 } else { rng.below(16) };
    let (ty, amount) = if plain {
        (0, 0)
    } else if rng.chance(2) {
        (rng.below(4), rng.pick(&[0, 1, 2, 15, 30, 31]))
    } else {
        (rng.below(4), rng.below(32))
    };
    let mut cpu = random_cpu(rng, 0x1008);
    let c = build(
        form,
        op,
        s,
        P {
            regs,
            imm8,
            rot,
            ty,
            amount,
        },
    );
    if form == 2 && !rng.chance(4) {
        let low = if rng.chance(2) {
            AMOUNTS[rng.below(15) as usize] & 0xFF
        } else {
            rng.below(256)
        };
        cpu.r[regs[3] as usize] = rng.next() & 0xFFFF_FF00 | low;
    }
    (cpu, c)
}

/// What a run of `(op, s)` cases must have exercised.
#[derive(Clone, Copy)]
struct Want {
    op: u32,
    s: bool,
    /// The operand is a plain register, so the shifter carry is C and logic ops never change C.
    plain: bool,
    /// The result can never be zero (`mvn` of a rotated 8-bit immediate).
    no_zero: bool,
}

fn expect(tag: &str, t: &Tally, n: u32, w: Want) {
    let arith = matches!(w.op, 2..=7 | 10 | 11);
    assert_eq!(t.cases, n, "{tag}: every case was compared");
    if (8..=11).contains(&w.op) {
        assert_eq!(t.reg_changed, 0, "{tag}: test ops write no register");
    } else {
        assert!(t.reg_changed > n / 3, "{tag}: rd changed: {t:?}");
    }
    if !w.s {
        assert_eq!(t.flags_changed, 0, "{tag}: no S, no flag write");
        return;
    }
    assert!(t.flags_changed > n / 4, "{tag}: flags changed: {t:?}");
    // arithmetic changes all four flags, logic ops keep V (and C too for a plain register)
    let bits = if arith {
        0b1111
    } else if w.plain {
        0b1100
    } else {
        0b1110
    };
    assert_eq!(t.bits_changed, bits, "{tag}: flag bits written: {t:?}");
    let set = if w.no_zero { 0b1011 } else { 0xF };
    assert_eq!(
        (t.seen_set, t.seen_clear),
        (set, 0xF),
        "{tag}: flags seen: {t:?}"
    );
}

fn seed(form: u32, op: u32, s: bool) -> u64 {
    0xA1_0000 | (form as u64) << 8 | (op as u64) << 1 | s as u64
}

#[test]
fn every_generic_arm_equals_dp_exec_on_random_states() {
    let n = (2000 * fuzz_cases(60) / 60).max(300) as u32;
    for form in 0..3 {
        for op in 0..16 {
            for s in [false, true] {
                let mut rng = Rng::new(seed(form, op, s));
                let mut t = Tally::default();
                for _ in 0..n {
                    let (cpu, c) = random_case(&mut rng, form, op, s, false);
                    check(&cpu, &c, &mut t).unwrap_or_else(|e| panic!("{form} {op} {s}: {e}"));
                }
                let w = Want {
                    op,
                    s,
                    plain: false,
                    no_zero: form == 0 && op == 15,
                };
                expect(&format!("form {form} op {op} s {s}"), &t, n, w);
            }
        }
    }
}

#[test]
fn the_shifter_equals_the_interpreter_on_every_amount_type_and_carry() {
    let mut t = Tally::default();
    for ty in 0..4 {
        for f in 0..16u8 {
            for &val in &VALS {
                for amount in 0..32 {
                    // movs r0, r1, <ty> #amount
                    let p = P {
                        regs: [0, 0, 1, 0],
                        ty,
                        amount,
                        ..P::default()
                    };
                    check(&cpu_with(&[(1, val)], f), &build(1, 13, true, p), &mut t).unwrap();
                }
                for &reg in &AMOUNTS {
                    // movs r0, r1, <ty> r2
                    let p = P {
                        regs: [0, 0, 1, 2],
                        ty,
                        ..P::default()
                    };
                    check(
                        &cpu_with(&[(1, val), (2, reg)], f),
                        &build(2, 13, true, p),
                        &mut t,
                    )
                    .unwrap();
                }
            }
        }
    }
    assert_eq!(t.cases as usize, 4 * 16 * VALS.len() * (32 + AMOUNTS.len()));
    assert_eq!((t.seen_set, t.seen_clear), (0xF, 0xF));
    assert!(t.flags_changed > 5000 && t.reg_changed > 5000, "{t:?}");
}

#[test]
fn every_opcode_equals_the_interpreter_on_edge_operand_pairs_and_all_flags() {
    for op in 0..16 {
        for s in [false, true] {
            let mut t = Tally::default();
            for f in 0..16u8 {
                for &a in &VALS {
                    for &b in &VALS {
                        // op r0, r1, r2 (lsl #0)
                        let c = build(
                            1,
                            op,
                            s,
                            P {
                                regs: [0, 1, 2, 0],
                                ..P::default()
                            },
                        );
                        check(&cpu_with(&[(1, a), (2, b)], f), &c, &mut t).unwrap();
                    }
                }
            }
            let w = Want {
                op,
                s,
                plain: true,
                no_zero: false,
            };
            expect(&format!("edge op {op} s {s}"), &t, 16 * 169, w);
        }
    }
}
