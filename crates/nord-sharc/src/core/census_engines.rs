//! JIT and AOT against the interpreter on the census cases (D152).
//!
//! The five semantics closed in D151 (8-bit signed shift-immediate count, MV/MS conditions,
//! SIMD `Rx <-> Sy` swap conditions, long-word load into ASTAT/STKY, SIMD conditional load into a
//! non-data register) live in the interpreter. The other engines reach them like this:
//! * JIT: `run_jit` falls back to [`Core::step`] for `ShiftImm`, conditional moves, swaps, non-data
//!   registers and long-word moves (`jit.rs`: only `cond 31`, `ureg < 16`, no `LW` are lowered), so
//!   the test replays every census case on both and compares the dumped state word for word.
//! * AOT: only `cond 31` compute, `LoadImm`, `RegMove` and the bit-operation shifts `0xC0`, `0xC4`,
//!   `0xC8` are native (`aot_lower.rs`); everything else is an interpreter item. The test checks
//!   the classification of every census instruction and `Pe::shift` against the interpreter.
//!
//! The census slot code cannot run as a generated AOT block (blocks are generated from firmware
//! ranges), hence the classification check instead of a lockstep run.
use super::aot_lower::InsnClass;
use super::aot_pe::Pe;
use super::census::{run_cases_on, Engine};
use super::support::{core_with, CODE};
use super::Core;

const FIXTURE: &str = include_str!("../../../../docs/research/sharc-seq/census_fixture.txt");
const CASES: &str = include_str!("../../../../docs/research/sharc-seq/census_cases.txt");
const EXTRA_CASES: &str =
    include_str!("../../../../docs/research/sharc-seq/census_extra_cases.txt");

/// Distinct instruction parcels (hex) of a census case file, in file order.
fn instructions(cases: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    cases
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|t| t.len() >= 4 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .filter(|t| seen.insert(t.to_string()))
        .map(str::to_string)
        .collect()
}

fn parcels(hex: &str) -> Vec<u16> {
    (0..hex.len() / 4)
        .map(|i| u16::from_str_radix(&hex[4 * i..4 * i + 4], 16).unwrap_or(1))
        .collect()
}

/// Every census case on the JIT gives the same dumped state and scratch words as the interpreter.
#[cfg(feature = "jit")]
#[test]
fn jit_equals_the_interpreter_on_every_census_case() {
    let mut total = 0;
    for cases in [CASES, EXTRA_CASES] {
        let a = run_cases_on(Engine::Interp, FIXTURE, cases).unwrap();
        let b = run_cases_on(Engine::Jit, FIXTURE, cases).unwrap();
        assert_eq!(a.lines().count(), b.lines().count());
        for (k, (x, y)) in a.lines().zip(b.lines()).enumerate() {
            assert_eq!(x, y, "case {k} differs between interpreter and JIT");
            total += 1;
        }
    }
    assert!(total >= 1500, "{total} cases replayed");
}

/// The interpreter replay itself still works through the engine selector.
#[test]
fn the_interpreter_engine_replays_the_cases() {
    let out = run_cases_on(Engine::Interp, FIXTURE, EXTRA_CASES).unwrap();
    assert!(out.lines().count() > 400);
    assert!(!out.contains("FAULT"));
}

/// Instructions of the five D151 shapes are never lowered natively by the AOT: conditional
/// MV/MS forms, swaps, long-word and non-data register moves and the count-style shifts stay
/// interpreter items.
#[test]
fn the_aot_lowers_none_of_the_five_closed_shapes_natively() {
    let (mut native, mut interp, mut brk, mut checked) = (0, 0, 0, 0);
    for hex in instructions(CASES)
        .into_iter()
        .chain(instructions(EXTRA_CASES))
    {
        let ps = parcels(&hex);
        if ps.is_empty() || ps.len() > 3 {
            continue;
        }
        let c = core_with(&ps);
        let shape = c.aot_describe_range(CODE, CODE)[0].2.clone();
        let (_, class) = c.aot_insn_class(CODE);
        match class {
            InsnClass::Native => native += 1,
            InsnClass::Interp(_) => interp += 1,
            InsnClass::Break(_) => brk += 1,
        }
        let shape_five = shape.contains("RegSwap")
            || shape.contains("cond: 5,")
            || shape.contains("cond: 6,")
            || shape.contains("cond: 21,")
            || shape.contains("cond: 22,");
        if shape_five {
            checked += 1;
            assert!(
                !matches!(class, InsnClass::Native),
                "{hex} ({shape}) lowered natively"
            );
        }
    }
    assert!(checked > 0, "no instruction of the closed shapes found");
    assert!(native + interp + brk > 100);
}

/// `Pe::shift` (the AOT's native `BSET` / `BCLR` / `BTGL`) against the interpreter's shifter for
/// in-range, out-of-range and negative bit numbers: result, SZ SV SS and the SF bit of ASTAT.
#[test]
fn the_aot_bit_shifts_equal_the_interpreter() {
    let rx_values = [0u32, 1, 0x8000_0000, 0xFFFF_FFFF, 0x1234_5678];
    let bits = [0u32, 1, 5, 31, 32, 33, 255, 256, 0x8000_0000, u32::MAX];
    for op in [0xC0u64, 0xC4, 0xC8] {
        for &rx in &rx_values {
            for &ry in &bits {
                let mut a = Core::new(&[]);
                a.r[1] = rx;
                a.sys.astat_extra[0] = 1 << 14; // SF set: every shifter op clears it
                let mut b = Core::new(&[]);
                b.r[1] = rx;
                b.sys.astat_extra[0] = 1 << 14;
                assert!(a.shifter_op(op, 2, rx, ry));
                let mut pe = Pe::load(&mut b, false);
                pe.shift(op, 2, 1, ry);
                let st = pe.sticky();
                b.aot_pe_end(false, st);
                assert_eq!(a.r, b.r, "op {op:#x} rx {rx:#x} ry {ry:#x}");
                assert_eq!(a.flags, b.flags, "flags op {op:#x} rx {rx:#x} ry {ry:#x}");
                assert_eq!(a.sys.astat_extra[0], b.sys.astat_extra[0]);
            }
        }
    }
}
