//! Acceptance tests of the shifter, shift-immediate, dual add/subtract and multifunction handlers,
//! of the census instructions, and of the bail rule across both elements.
//!
//! OWNER: W2-FIXED, tests. Same rig and same rules as `diff_ops.rs`: real instruction words, the
//! registered handler against `Core::step` on full states (a case it bails on is re-run through
//! the handler without the bail), minimums asserted per kind and opcode.
use super::diff::Case;
use super::diff_sweep::{sweep, Tables};
use super::fixture::{random_programs, reg, t2a, t6n};
use super::kinds::{all_kinds, op8, word, XDUAL_K, XMF2_K, XMF3_K, XMRT_K, XSHIFTI_K, XSHIFT_K};
use super::SHIFT_OPS;
use crate::core::uop::lower::guard::class_of;
use crate::core::uop::types::{Exit, GuardClass, Uop, UopKind};
use crate::core::uop::verify::gen::{census_encodings, Rng};
use crate::core::Core;
use crate::visa::Kind;
use std::collections::BTreeSet;

/// Register values at the edges of the shifter: zero, one, all ones, the sign bit and its
/// neighbours, alternating bits.
const RX: [u32; 6] = [0, 1, u32::MAX, 0x8000_0000, 0x7FFF_FFFF, 0xA5A5_5A5A];

/// Shift amounts and bit field descriptors (a position and a length in the low bits): zero and
/// one, both signs around 8, 32 and 64, descriptors with a length and a position.
const RY: [u32; 14] = [
    0,
    1,
    8,
    31,
    32,
    33,
    0xFFFF_FFFF,
    0xFFFF_FFF8,
    0xFFFF_FFE0,
    0xFFFF_FFDF,
    0x8000_0000,
    0x0820,
    0x1F20,
    0x201F,
];

/// The condition of a random program: TRUE for two thirds, else any condition code.
fn cond(rng: &mut Rng) -> u8 {
    if rng.below(3) == 0 {
        rng.below(31) as u8
    } else {
        31
    }
}

#[test]
fn shifter_ops_equal_the_interpreter() {
    let mut rng = Rng::new(0xC1);
    // the modelled opcodes, and the codes the family leaves alone (bit FIFO forms, other units)
    let mut programs = random_programs(2, &SHIFT_OPS, 8, &mut rng);
    programs.extend(random_programs(
        2,
        &[0x50, 0x58, 0x70, 0x74, 0x7C, 0x01, 0xFF],
        1,
        &mut rng,
    ));
    let mut ops = BTreeSet::new();
    let t = sweep(&programs, 4, &mut |c| {
        if c.changed() {
            ops.insert(op8(c.a));
        }
    });
    assert_eq!(
        t.declined,
        7 * 2 * 4,
        "the bit FIFO forms stay with the interpreter"
    );
    assert_eq!(
        ops.into_iter().collect::<Vec<_>>(),
        SHIFT_OPS,
        "every opcode changed data somewhere"
    );
    let [ok, slow, changed, pey, raised, _] = t.of(XSHIFT_K);
    assert_eq!(
        (ok, slow, raised),
        (21 * 8 * 8, 0, 0),
        "a shifter never raises an exception"
    );
    assert!(changed >= ok / 2 && pey >= ok / 4, "{:?}", t.of(XSHIFT_K));
}

#[test]
fn shifter_edges_equal_the_interpreter() {
    let tables = Tables::new();
    // [flag false, flag true] counts of SZ, SV, SS after a case that changed the data
    let mut flags = [[0usize; 2]; 3];
    let mut ran = 0;
    for op in SHIFT_OPS {
        let prog = t2a(31, 2 << 20 | word(op, 3, 1, 2));
        for (i, &rx) in RX.iter().enumerate() {
            for (j, &ry) in RY.iter().enumerate() {
                let setup = move |c: &mut Core| {
                    (c.r[1], c.r[2], c.s[1], c.s[2]) = (rx, ry, ry, rx);
                };
                let seed = 100 + (i * RY.len() + j) as u64;
                let (c, forced) = tables
                    .run(&prog, seed, true, &setup)
                    .unwrap_or_else(|e| panic!("{e}"))
                    .expect("lowered");
                assert_eq!((c.exit, forced), (Exit::Ok, false));
                ran += 1;
                if c.changed() {
                    for (k, f) in [c.after.flags.sz, c.after.flags.sv, c.after.flags.ss]
                        .into_iter()
                        .enumerate()
                    {
                        flags[k][usize::from(f)] += 1;
                    }
                }
            }
        }
    }
    assert_eq!(ran, 21 * RX.len() * RY.len());
    assert!(
        flags.iter().flatten().all(|&n| n >= 30),
        "SZ, SV and SS took both values: {flags:?}"
    );
}

#[test]
fn shift_immediate_equals_the_interpreter() {
    const IMM: [u32; 12] = [
        0, 1, 0x1F, 0x20, 0x3F, 0x80, 0xFF, 0x100, 0x520, 0xA3C, 0xFE0, 0xFFF,
    ];
    let mut rng = Rng::new(0xD1);
    let mut programs = Vec::new();
    for op6 in 0..64u8 {
        for &d in &IMM {
            let (c, n, x) = (cond(&mut rng), reg(&mut rng) as u8, reg(&mut rng) as u8);
            programs.push(t6n(c, op6, n, x, d));
        }
    }
    let mut ops = BTreeSet::new();
    let t = sweep(&programs, 2, &mut |c| {
        if c.changed() {
            ops.insert(c.a as u8);
        }
    });
    let [ok, slow, changed, pey, _, _] = t.of(XSHIFTI_K);
    assert!(
        t.declined > 100,
        "the bit FIFO forms and the codes without an immediate form stay"
    );
    assert!(
        ok >= 300 && slow == 0 && changed >= ok / 3 && pey >= ok / 5,
        "{:?}",
        t.of(XSHIFTI_K)
    );
    assert!(
        ops.is_subset(&SHIFT_OPS.iter().copied().collect()),
        "{ops:x?}"
    );
    assert!(
        ops.len() >= 10,
        "{} opcodes with an immediate form changed data: {ops:x?}",
        ops.len()
    );
}

#[test]
fn dual_and_multifunction_words_equal_the_interpreter() {
    let mut rng = Rng::new(0xE1);
    let mut programs = Vec::new();
    // the dual add/subtract `Ra = Rx + Ry, Rs = Rx - Ry`: bits 19..16 are 7, Rs 15..12, Ra 11..8
    for _ in 0..40 {
        let c =
            7 << 16 | rng.below(16) << 12 | rng.below(16) << 8 | reg(&mut rng) << 4 | reg(&mut rng);
        programs.push(t2a(cond(&mut rng), c));
    }
    // the multiplier plus ALU forms: op6 in bits 21..16, every group and every low pair
    for group in 1..=5u32 {
        for low in 0..=2u32 {
            for _ in 0..3 {
                let (rm, ra, operands) = (rng.below(16), rng.below(16), rng.below(256));
                let c = 1 << 22 | (group << 2 | low) << 16 | rm << 12 | ra << 8 | operands;
                programs.push(t2a(cond(&mut rng), c));
            }
        }
    }
    // the triple form: Rs 19..16, Rm 15..12, Ra 11..8
    for _ in 0..40 {
        let c = 1 << 22 | 1 << 21 | rng.below(16) << 16 | rng.below(16) << 12 | rng.below(16) << 8;
        programs.push(t2a(cond(&mut rng), c | rng.below(256)));
    }
    // the rest of the multifunction word space: transfers, float forms, codes without an operation
    for _ in 0..100 {
        programs.push(t2a(cond(&mut rng), 1 << 22 | (rng.next_u32() & 0x3F_FFFF)));
    }
    let (mut op6s, mut duals) = (BTreeSet::new(), BTreeSet::new());
    let t = sweep(&programs, 3, &mut |c| {
        if c.changed() && c.kind == XMF2_K {
            op6s.insert(c.a >> 16 & 63);
        }
        if c.changed() && c.kind == XDUAL_K {
            duals.insert(c.a >> 12 & 15);
        }
    });
    let want: BTreeSet<u32> = (1..=5u32)
        .flat_map(|g| (0..=2u32).map(move |l| g << 2 | l))
        .collect();
    assert_eq!(
        op6s, want,
        "every multiplier plus ALU form changed data somewhere"
    );
    assert!(
        duals.len() >= 8,
        "dual add/subtract with {} different Rs",
        duals.len()
    );
    assert!(
        t.declined > 50,
        "float forms and codes without an operation stay: {}",
        t.declined
    );
    for k in [XDUAL_K, XMF2_K, XMF3_K] {
        let [ok, slow, changed, pey, raised, forced] = t.of(k);
        assert!(
            ok >= 100 && changed >= ok / 2 && pey >= ok / 4,
            "kind {k:#x}: {:?}",
            t.of(k)
        );
        assert!(
            slow == raised && forced == slow && raised >= 5,
            "kind {k:#x}: {:?}",
            t.of(k)
        );
    }
    assert!(
        t.of(XMRT_K)[0] > 0,
        "the transfer forms in the same space are lowered too"
    );
}

#[test]
fn census_instructions_equal_the_interpreter() {
    let pool = census_encodings(|k, _| matches!(k, Kind::T2a | Kind::T2b | Kind::T6n));
    assert!(pool.len() >= 120, "{} census instructions", pool.len());
    let programs: Vec<Vec<u16>> = pool.iter().map(|p| p.to_vec()).collect();
    let t = sweep(&programs, 2, &mut |_| ());
    let kinds: Vec<u16> = all_kinds()
        .into_iter()
        .filter(|&k| t.of(k)[0] > 0)
        .collect();
    assert!(
        t.ok() >= 200 && kinds.len() >= 8,
        "ran {} cases of {} kinds: {t:?}",
        t.ok(),
        kinds.len()
    );
    assert!(
        t.declined > 0,
        "the float and bit FIFO instructions of the census stay with the interpreter"
    );
}

/// `IF cond R3 = R1 + R2` on both elements with the registers of PEx and PEy set and the zero flag
/// of each element set, and what the case did.
fn add_case(t: &Tables, cond: u8, x: (u32, u32), y: (u32, u32), az: (bool, bool)) -> Case {
    let setup = move |c: &mut Core| {
        (c.r[1], c.r[2], c.s[1], c.s[2]) = (x.0, x.1, y.0, y.1);
        (c.flags.az, c.flags_y.az) = az;
    };
    t.run(&t2a(cond, word(0x01, 3, 1, 2)), 3, true, &setup)
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("lowered")
        .0
}

#[test]
fn an_exception_on_an_element_that_runs_bails_the_whole_instruction_untouched() {
    const MAX: u32 = 0x7FFF_FFFF;
    let t = Tables::new();
    let (fine, over) = ((1, 2), (MAX, 1));
    // IF TRUE: either element, or both, overflowing bails; the rig has checked that the core was
    // left as it was before the interpreter ran the instruction
    for (x, y, bails) in [
        (fine, fine, false),
        (fine, over, true),
        (over, fine, true),
        (over, over, true),
    ] {
        let c = add_case(&t, 31, x, y, (false, false));
        assert_eq!(
            (c.exit == Exit::Slow, c.raised),
            (bails, bails),
            "x {x:?} y {y:?}"
        );
        assert_eq!(c.ran, (true, true));
    }
    // IF EQ: an element whose condition fails neither runs nor raises
    let c = add_case(&t, 0, fine, over, (true, false));
    assert_eq!(
        (c.exit, c.raised, c.ran),
        (Exit::Ok, false, (true, false)),
        "PEy overflows but does not run"
    );
    let c = add_case(&t, 0, over, fine, (false, true));
    assert_eq!(
        (c.exit, c.raised, c.ran),
        (Exit::Ok, false, (false, true)),
        "PEx overflows but does not run"
    );
    let c = add_case(&t, 0, fine, over, (true, true));
    assert_eq!(
        (c.exit, c.raised, c.ran),
        (Exit::Slow, true, (true, true)),
        "PEy runs and overflows"
    );
    let c = add_case(&t, 0, over, over, (false, false));
    assert_eq!(
        (c.exit, c.raised, c.ran),
        (Exit::Ok, false, (false, false)),
        "neither runs"
    );
    assert!(
        !c.changed(),
        "a compute that runs on no element changes no data"
    );
}

#[test]
fn the_handler_without_the_bail_runs_the_overflow_and_equals_the_interpreter() {
    // R3 = R1 + R2 overflowing on PEx, 5 + 6 on PEy: `Tables::run` re-runs the bailing case
    let t = Tables::new();
    let setup = |c: &mut Core| (c.r[1], c.r[2], c.s[1], c.s[2]) = (0x7FFF_FFFF, 1, 5, 6);
    let (c, forced) = t
        .run(&t2a(31, word(0x01, 3, 1, 2)), 9, true, &setup)
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("lowered");
    assert_eq!(
        (c.exit, c.raised, forced, c.after.s[3]),
        (Exit::Slow, true, true, 11)
    );
    assert!(
        matches!(c.after.r[3], 0x8000_0000 | 0x7FFF_FFFF),
        "wrapped or saturated: {:#x}",
        c.after.r[3]
    );
}

#[test]
fn every_fixed_uop_is_an_arithmetic_guard_class_block() {
    // a block with one of these uops must not start while the arithmetic exception interrupt is
    // enabled (guard row `Arith`): the handlers bail on an exception, they never raise it
    for k in all_kinds() {
        let u = Uop::new(UopKind(k), 1, 0, 0, 31);
        assert_eq!(class_of(&[u]), GuardClass::Arith, "kind {k:#x}");
    }
}
