//! Differential test of the kernel path (`exec_kernels`: element view plus exact kernels) against
//! the two oracles, on random full element states: the interpreter (`Core::exec_compute_op` on
//! PEx, then on the swapped view for PEy, exactly what `Core::step` does in SIMD mode) and
//! `Pe::exec` through [`super::exec_both`] where `Pe` implements the op.
//!
//! OWNER: W1-KERN, test support and the acceptance tests of the package. Everything the ops can
//! write is compared, hidden state included: the data registers of both elements, the float
//! extension tag and low byte of every register, the ALU flags (all of `Flags`), the ASTAT extra
//! bits (AF, MN..MI, CACC), STKY and the interrupt latches the sticky bits raise. The full
//! `exact_diff` runs on every eighth case (it also compares the 4 MiB SRAM image).
//!
//! Non-vacuous (design P10): [`Tally`] counts cases per op and mode, cases whose state changed,
//! hidden-extension changes, sticky cases and every flag outcome, and the tests assert minimums;
//! `the_comparison_sees_every_field_it_claims_to_compare` perturbs each compared field and
//! demands a difference, so the comparison itself cannot be blind.
use super::{exec_both, exec_kernels, kernels_support};
use crate::core::aot_pe::pe_supports;
use crate::core::decode_cache::ComputeOp;
use crate::core::float::{MODE1_ALUSAT, MODE1_RND32, MODE1_TRUNC};
use crate::core::support::core_with;
use crate::core::types::Flags;
use crate::core::uop::kernels::grid;
use crate::core::uop::verify::compare::exact_diff;
use crate::core::uop::verify::gen::Rng;
use crate::core::{Core, MODE1_PEYEN};
use std::collections::BTreeMap;

/// The float ALU opcodes the kernels implement (`kernels_support`).
/// VERIFIED: the opcode numbers of `Core::float_alu`; the run asserts `kernels_support` and an
/// interpreter step of every one of them.
const ALU_OPS: [u8; 19] = [
    0x81, 0x82, 0x89, 0x8A, 0xA1, 0xA2, 0xB0, 0xA5, 0xBD, 0xC9, 0xCA, 0xCD, 0xD9, 0xDA, 0xDD, 0xE0,
    0xE1, 0xE2, 0xE3,
];

/// The MODE1 bits the float ops read (ALUSAT, TRUNC, RND32), drawn at random for every case.
const MODE_BITS: u32 = MODE1_ALUSAT | MODE1_TRUNC | MODE1_RND32;
/// PEYEN: the compute field also runs on PEy.
const PEYEN: u32 = MODE1_PEYEN;

/// Every state the ops of the package can read or write, and the means to load it into a core.
struct State {
    r: [u32; 16],
    s: [u32; 16],
    tag: [u32; 16],
    lo: [u8; 16],
    tag_y: [u32; 16],
    lo_y: [u8; 16],
    flags: Flags,
    flags_y: Flags,
    astat: [u32; 2],
    stky: [u32; 2],
    mode1: u32,
    /// The ALU flags start clear, so a flag that is set afterwards was set by the op.
    flags_clear: bool,
}

impl State {
    fn random(rng: &mut Rng, simd: bool) -> State {
        let flags_clear = rng.next_u64().is_multiple_of(4);
        let mut st = State {
            r: [0; 16],
            s: [0; 16],
            tag: [0; 16],
            lo: [0; 16],
            tag_y: [0; 16],
            lo_y: [0; 16],
            flags: if flags_clear {
                Flags::default()
            } else {
                random_flags(rng)
            },
            flags_y: if flags_clear {
                Flags::default()
            } else {
                random_flags(rng)
            },
            astat: [rng.next_u32(), rng.next_u32()],
            stky: [rng.next_u32() & 0x3FF, rng.next_u32() & 0x3FF],
            mode1: rng.next_u32() & MODE_BITS | if simd { PEYEN } else { 0 },
            flags_clear,
        };
        for k in 0..16 {
            (st.r[k], st.tag[k], st.lo[k]) = register(rng);
            (st.s[k], st.tag_y[k], st.lo_y[k]) = register(rng);
        }
        st
    }

    fn apply(&self, c: &mut Core) {
        (c.r, c.s, c.fx.tag, c.fx.lo) = (self.r, self.s, self.tag, self.lo);
        (c.fx_y.tag, c.fx_y.lo) = (self.tag_y, self.lo_y);
        (c.flags, c.flags_y) = (self.flags, self.flags_y);
        (c.sys.astat_extra, c.sys.stky, c.mode1) = (self.astat, self.stky, self.mode1);
        // the interrupt latches the sticky bits raise start clear in every case (the cores are
        // reused, and `Pe` runs fewer ops than the interpreter)
        (c.irq.irptl, c.irq.lirptl, c.irq.due) = (0, 0, 0);
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

/// One register: a float operand with a valid or a stale extension, a scale, a grid value or
/// random bits. Returns the 32-bit register, its extension tag and its extension low byte.
fn register(rng: &mut Rng) -> (u32, u32, u8) {
    let v = match rng.next_u64() % 8 {
        0..=3 => grid::operand(rng),
        4 | 5 => {
            let k = match rng.next_u64() % 8 {
                0..=4 => (rng.next_u64() % 80) as i32 - 40,
                5 => 127 + (rng.next_u64() % 200) as i32 - 100,
                6 => [i32::MIN, i32::MAX, -1, 1 << 30][(rng.next_u64() % 4) as usize],
                _ => rng.next_u32() as i32,
            };
            u64::from(k as u32) << 8 | rng.next_u64() & 0xFF
        }
        6 => {
            let g = grid::grid();
            g[(rng.next_u64() % g.len() as u64) as usize]
        }
        _ => rng.next_u64() & 0xFF_FFFF_FFFF,
    };
    let r = (v >> 8) as u32;
    if !rng.next_u64().is_multiple_of(3) {
        (r, r, v as u8)
    } else {
        // a stale extension: written by something that does not know about it
        (
            r,
            r.wrapping_add(1 + (rng.next_u64() % 5) as u32),
            rng.next_u64() as u8,
        )
    }
}

/// The first field in which the two cores differ in anything an op of the package can write.
fn precise_diff(a: &Core, b: &Core) -> Option<String> {
    macro_rules! same {
        ($name:literal, $x:expr, $y:expr) => {
            if $x != $y {
                return Some(format!("{}: {:x?} vs {:x?}", $name, $x, $y));
            }
        };
    }
    same!("r", a.r, b.r);
    same!("s", a.s, b.s);
    same!("tag", a.fx.tag, b.fx.tag);
    same!("lo", a.fx.lo, b.fx.lo);
    same!("tag_y", a.fx_y.tag, b.fx_y.tag);
    same!("lo_y", a.fx_y.lo, b.fx_y.lo);
    same!("flags", a.flags, b.flags);
    same!("flags_y", a.flags_y, b.flags_y);
    same!("astat_extra", a.sys.astat_extra, b.sys.astat_extra);
    same!("stky", a.sys.stky, b.sys.stky);
    same!("mode1", a.mode1, b.mode1);
    same!("irptl", a.irq.irptl, b.irq.irptl);
    same!("lirptl", a.irq.lirptl, b.irq.lirptl);
    same!("irq_due", a.irq.due, b.irq.due);
    None
}

/// What the interpreter does for one compute field (`Core::step`): PEx, then PEy in the swapped
/// view when PEy is enabled.
fn interpret(c: &mut Core, op: ComputeOp, simd: bool) {
    assert!(c.exec_compute_op(op), "{op:?}");
    if simd {
        c.swap_pe();
        assert!(c.exec_compute_op(op), "{op:?} on PEy");
        c.swap_pe();
    }
}

/// What a run covered.
#[derive(Debug, Default)]
struct Tally {
    cases: usize,
    changed: usize,
    hidden_changed: usize,
    sticky: usize,
    /// Cases that latched an arithmetic-exception interrupt (IRPTL).
    latched: usize,
    exact_checked: usize,
    pe_checked: usize,
    per_op: BTreeMap<(u16, bool), usize>,
    /// ALU-flag outcomes (AZ, AN, AV, AS, AI) of the cases that started with clear flags.
    flags: [usize; 5],
    from_clear: usize,
}

fn op_id(op: ComputeOp) -> u16 {
    match op {
        ComputeOp::FloatMul { .. } => 0x130,
        ComputeOp::FloatAlu { op, .. } => u16::from(op),
        _ => u16::MAX,
    }
}

/// The ops with random operand registers: `n` and the operands collide often (small register
/// numbers) so aliasing is exercised as much as distinct registers.
fn ops_for(rng: &mut Rng) -> Vec<ComputeOp> {
    let mut reg = || {
        if rng.next_u64() & 1 == 0 {
            (rng.next_u64() % 16) as u8
        } else {
            (rng.next_u64() % 3) as u8
        }
    };
    let (n, x, y) = (reg(), reg(), reg());
    let mut v: Vec<ComputeOp> = ALU_OPS
        .iter()
        .map(|&op| ComputeOp::FloatAlu { op, n, x, y })
        .collect();
    v.push(ComputeOp::FloatMul { n, x, y });
    v
}

fn run(seed: u64, rounds: usize) -> Tally {
    let mut rng = Rng::new(seed);
    // the interpreter run, the `Pe::exec` run, the kernel run and the state before any of them
    let mut cores = [(); 4].map(|()| core_with(&[0x0001]));
    let mut t = Tally::default();
    for round in 0..rounds {
        for simd in [false, true] {
            let state = State::random(&mut rng, simd);
            for op in ops_for(&mut rng) {
                assert!(kernels_support(op), "{op:?}");
                let [interp, pe, kern, before] = &mut cores;
                for c in [&mut *interp, &mut *pe, &mut *kern, &mut *before] {
                    state.apply(c);
                }
                interpret(interp, op, simd);
                assert!(exec_kernels(kern, op, simd), "{op:?}");
                let ctx = format!("round {round} simd {simd} {op:?}");
                if let Some(d) = precise_diff(interp, kern) {
                    panic!("kernels vs interpreter, {ctx}: {d}");
                }
                if pe_supports(op) {
                    assert!(exec_both(pe, op, simd));
                    if let Some(d) = precise_diff(interp, pe) {
                        panic!("Pe::exec vs interpreter, {ctx}: {d}");
                    }
                    t.pe_checked += 1;
                }
                if t.cases % 8 == 0 {
                    assert_eq!(exact_diff(interp, kern), None, "{ctx}");
                    t.exact_checked += 1;
                }
                t.cases += 1;
                *t.per_op.entry((op_id(op), simd)).or_default() += 1;
                t.changed += usize::from(precise_diff(before, kern).is_some());
                t.hidden_changed += usize::from(
                    (before.fx.tag, before.fx.lo, before.fx_y.tag, before.fx_y.lo)
                        != (kern.fx.tag, kern.fx.lo, kern.fx_y.tag, kern.fx_y.lo),
                );
                t.sticky += usize::from(before.sys.stky != kern.sys.stky);
                t.latched += usize::from(kern.irq.irptl != 0);
                if state.flags_clear && !matches!(op, ComputeOp::FloatMul { .. }) {
                    t.from_clear += 1;
                    let f = kern.flags;
                    for (n, set) in t.flags.iter_mut().zip([f.az, f.an, f.av, f.as_, f.ai]) {
                        *n += usize::from(set);
                    }
                }
            }
        }
    }
    t
}

#[test]
fn kernel_ops_equal_interpreter_and_pe_exec() {
    let rounds = 220;
    let t = run(0xD1FF_0001, rounds);
    // every op, on one element and on both, ran `rounds` times
    assert_eq!(t.per_op.len(), 20 * 2, "{:?}", t.per_op);
    assert!(t.per_op.values().all(|&n| n == rounds), "{:?}", t.per_op);
    assert_eq!(t.cases, 20 * 2 * rounds);
    // the ops changed the state in nearly every case (a compare only changes flags and CACC)
    assert!(t.changed * 100 > t.cases * 95, "{t:?}");
    assert!(
        t.hidden_changed * 4 > t.cases,
        "extension tag and low byte were written: {t:?}"
    );
    assert!(t.sticky * 10 > t.cases, "sticky bits were raised: {t:?}");
    assert!(t.latched * 20 > t.cases, "interrupts were latched: {t:?}");
    assert!(
        t.exact_checked * 8 >= t.cases && t.pe_checked > t.cases / 2,
        "{t:?}"
    );
    // every ALU flag outcome was produced by an op, not left over from the initial state
    assert!(
        t.from_clear > 1000 && t.flags.iter().all(|&n| n >= 20),
        "{t:?}"
    );
}

#[test]
fn the_comparison_sees_every_field_it_claims_to_compare() {
    let mut rng = Rng::new(0xD1FF_0002);
    let state = State::random(&mut rng, true);
    type Perturb = fn(&mut Core);
    let perturb: [(&str, Perturb); 14] = [
        ("r", |c| c.r[5] ^= 1),
        ("s", |c| c.s[5] ^= 1),
        ("tag", |c| c.fx.tag[5] ^= 1),
        ("lo", |c| c.fx.lo[5] ^= 1),
        ("tag_y", |c| c.fx_y.tag[5] ^= 1),
        ("lo_y", |c| c.fx_y.lo[5] ^= 1),
        ("flags", |c| c.flags.sv = !c.flags.sv),
        ("flags_y", |c| c.flags_y.ac = !c.flags_y.ac),
        ("astat_extra", |c| c.sys.astat_extra[1] ^= 1 << 30),
        ("stky", |c| c.sys.stky[1] ^= 1),
        ("mode1", |c| c.mode1 ^= 1 << 13),
        ("irptl", |c| c.irq.irptl ^= 1 << 7),
        ("lirptl", |c| c.irq.lirptl ^= 1 << 7),
        ("irq_due", |c| c.irq.due ^= 1),
    ];
    for (name, f) in perturb {
        let (mut a, mut b) = (core_with(&[0x0001]), core_with(&[0x0001]));
        state.apply(&mut a);
        state.apply(&mut b);
        assert_eq!(
            precise_diff(&a, &b),
            None,
            "identical states differ before {name}"
        );
        f(&mut b);
        let d = precise_diff(&a, &b).unwrap_or_else(|| panic!("a change of {name} is invisible"));
        assert!(d.starts_with(name), "{name}: {d}");
    }
}

#[test]
fn a_kernel_that_is_wrong_in_one_bit_is_caught_by_the_same_comparison() {
    // run the kernel path on a copy and corrupt one hidden extension byte afterwards: the
    // comparison against the interpreter must fail, which is what a wrong kernel would do
    let mut rng = Rng::new(0xD1FF_0003);
    let state = State::random(&mut rng, false);
    let op = ComputeOp::FloatAlu {
        op: 0x81,
        n: 3,
        x: 1,
        y: 2,
    };
    let (mut interp, mut kern) = (core_with(&[0x0001]), core_with(&[0x0001]));
    state.apply(&mut interp);
    state.apply(&mut kern);
    interpret(&mut interp, op, false);
    assert!(exec_kernels(&mut kern, op, false));
    assert_eq!(precise_diff(&interp, &kern), None);
    kern.fx.lo[3] ^= 0x10;
    assert!(precise_diff(&interp, &kern).unwrap().starts_with("lo"));
}
