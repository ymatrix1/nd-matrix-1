//! The lowering against the committed census and against every 16-bit instruction: the float
//! family takes exactly the float computes it has a handler for.
//!
//! OWNER: W2-FLOAT, test only. The expectation is written here as a list of opcodes, separately
//! from `select`, so a table slip on either side shows. The corpus is `gen::census_encodings`
//! (the instructions the ADSP-21489 simulator ran, `docs/research/sharc-seq/`), plus a scan of
//! all 65,536 single-parcel instructions, which holds every 16-bit compute (Type 2c).
use super::FloatFamily;
use crate::core::decode_cache::{ComputeOp, DecodedOp};
use crate::core::support::{core_with, load, CODE};
use crate::core::uop::config::UopConfig;
use crate::core::uop::lower::builder::BlockBuilder;
use crate::core::uop::lower::decode::decode_at;
use crate::core::uop::lower::{Family, LowerCtx};
use crate::core::uop::verify::gen::census_encodings;
use crate::core::Core;

/// Float ALU opcodes with a handler: add, subtract, average, compare, pass, negate, abs, copysign,
/// min, max, clip, round, scale, fix, trunc, float (both scale forms) and the two seed ops.
/// VERIFIED: the opcode numbers of `Core::float_alu`, written down independently of `select`; the
/// two tests below fail on any opcode the family takes or misses against this list (max err 0).
const FLOAT_ALU: [u8; 21] = [
    0x81, 0x82, 0x89, 0x8A, 0xA1, 0xA2, 0xB0, 0xE0, 0xE1, 0xE2, 0xE3, 0xA5, 0xBD, 0xC9, 0xD9, 0xCD,
    0xDD, 0xCA, 0xDA, 0xC4, 0xC5,
];

/// Whether the family must take compute `op`.
fn must_take(op: ComputeOp) -> bool {
    match op {
        ComputeOp::FloatMul { .. } => true,
        ComputeOp::FloatAlu { op, .. } => FLOAT_ALU.contains(&op),
        // a float multiplier + float ALU multifunction: bits 21..19 = 0b011 (the fixed-point
        // multifunctions use other unit codes), written from `docs/specs/sharc-core.md`'s decode
        ComputeOp::Multifunction { c } => c >> 19 & 7 == 3,
        _ => false,
    }
}

/// `(taken, uop count)` of the instruction at `CODE` of `core`.
fn lowered(core: &mut Core, simd: bool) -> (bool, usize, DecodedOp) {
    let insn = decode_at(core, CODE);
    let cfg = UopConfig::default();
    let cx = LowerCtx {
        cfg: &cfg,
        stop_pc: None,
        simd,
        isa: false,
    };
    let mut b = BlockBuilder::new(CODE, simd, false);
    let taken = FloatFamily::lower(&cx, &insn, &mut b);
    (taken, b.len(), insn.op)
}

fn compute_of(op: DecodedOp) -> Option<ComputeOp> {
    match op {
        DecodedOp::ComputeCond { op, .. } | DecodedOp::ComputeBoth { op } => Some(op),
        _ => None,
    }
}

#[test]
fn every_opcode_the_family_takes_is_run_by_the_handler_tests() {
    use crate::core::uop::exec::h_float::support::ALU_OPS;
    let sorted = |v: &[u8]| {
        let mut v = v.to_vec();
        v.sort_unstable();
        v
    };
    assert_eq!(sorted(&FLOAT_ALU), sorted(&ALU_OPS));
}

#[test]
fn lower_takes_exactly_the_float_instructions_of_the_census() {
    let mut taken_floats = 0;
    let mut computes = 0;
    for parcels in census_encodings(|_, _| true) {
        let mut core = core_with(&[parcels, &[0x0001]].concat());
        let (taken, uops, op) = lowered(&mut core, false);
        let want = compute_of(op).is_some_and(must_take);
        assert_eq!(
            (taken, uops),
            (want, usize::from(want)),
            "{parcels:x?} {op:?}"
        );
        computes += usize::from(compute_of(op).is_some());
        taken_floats += usize::from(taken);
    }
    assert!(
        computes > 25,
        "the census holds {computes} plain compute instructions"
    );
    assert!(
        taken_floats > 8,
        "the census holds {taken_floats} float compute instructions the family takes"
    );
}

#[test]
fn lower_takes_exactly_the_float_computes_among_all_single_parcel_instructions() {
    let mut core = core_with(&[0, 0, 0]);
    let (mut taken_n, mut short_computes) = (0, 0);
    for p in 0..=u16::MAX {
        load(&mut core, CODE, &[p, 0, 0]);
        core.invalidate_code_cache();
        let insn = decode_at(&mut core, CODE);
        if insn.next_pc - CODE != 1 {
            continue;
        }
        let (taken, uops, op) = lowered(&mut core, p & 1 == 1);
        let want = compute_of(op).is_some_and(must_take);
        assert_eq!((taken, uops), (want, usize::from(want)), "{p:#06x} {op:?}");
        short_computes += usize::from(matches!(op, DecodedOp::ComputeBoth { .. }));
        taken_n += usize::from(taken);
    }
    assert!(short_computes > 100, "{short_computes} 16-bit computes");
    assert!(taken_n > 50, "{taken_n} 16-bit float computes are taken");
}
