//! Hand-checked behaviour of the data-processing arms (W1-XALU). Every expectation is written
//! down from the ARM architecture rule it exercises, not taken from the interpreter, so the
//! differential suite (`diff.rs`) and this file cannot share a mistake.

use super::*;

const C: u8 = 2;
const Z: u8 = 4;
const N: u8 = 8;
const V: u8 = 1;
const SENTINEL: u32 = 0xDEAD_0000;

/// `regs` as `r0..`, with `r15` a sentinel.
fn file(regs: &[u32]) -> Regs {
    let mut r = [0u32; 16];
    r[..regs.len()].copy_from_slice(regs);
    r[15] = SENTINEL;
    r
}

/// Run `u` on registers `regs` (r0 upwards) and flags `f`; returns (registers, flags).
fn run(u: Uop, regs: &[u32], f: u8) -> (Regs, u8) {
    let (mut r, mut f) = (file(regs), f);
    exec_dp(&mut r, &mut f, &u);
    assert_eq!(r[15], SENTINEL, "r15 is never touched");
    (r, f)
}

/// Form 0: `r0 = r1 <op> #imm`, `rot0` when the encoded rotation was 0.
fn imm(op: u32, s: bool, imm: u32, rot0: bool) -> Uop {
    Uop {
        rn: 1,
        sh: rot0 as u8,
        imm,
        ..Uop::new(k::dp(0, op, s), 0x1000)
    }
}

/// Form 1: `r0 = r1 <op> r2 <ty> #amount`.
fn shifted(op: u32, s: bool, ty: u8, amount: u8) -> Uop {
    Uop {
        rn: 1,
        rm: 2,
        sh: ty << 5 | amount,
        ..Uop::new(k::dp(1, op, s), 0x1000)
    }
}

/// Form 2: `r0 = r1 <op> r2 <ty> r3`.
fn by_reg(op: u32, s: bool, ty: u8) -> Uop {
    Uop {
        rn: 1,
        rm: 2,
        rs: 3,
        sh: ty << 5,
        ..Uop::new(k::dp(2, op, s), 0x1000)
    }
}

#[test]
fn add_and_sub_flags_follow_the_architecture_manual() {
    // adds: 0x7FFFFFFF + 1 overflows into N and V, 0xFFFFFFFF + 1 carries into Z and C
    let (r, f) = run(imm(4, true, 1, true), &[9, 0x7FFF_FFFF], 0);
    assert_eq!((r[0], f), (0x8000_0000, N | V));
    let (r, f) = run(imm(4, true, 1, true), &[9, 0xFFFF_FFFF], 0);
    assert_eq!((r[0], f), (0, Z | C));
    // subs: 5 - 5 = 0 with C (no borrow), 5 - 6 borrows (C clear, N set)
    let (r, f) = run(imm(2, true, 5, true), &[9, 5], 0);
    assert_eq!((r[0], f), (0, Z | C));
    let (r, f) = run(imm(2, true, 6, true), &[9, 5], C);
    assert_eq!((r[0], f), (0xFFFF_FFFF, N));
    // rsbs: 5 - 3 where the immediate is the minuend
    let (r, f) = run(imm(3, true, 5, true), &[9, 3], 0);
    assert_eq!((r[0], f), (2, C));
    // cmp does not write rd, cmn 1 + 0xFFFFFFFF sets Z and C
    let (r, f) = run(imm(10, true, 1, true), &[9, 1], 0);
    assert_eq!((r[0], f), (9, Z | C));
    let (r, f) = run(imm(11, true, 1, true), &[9, 0xFFFF_FFFF], 0);
    assert_eq!((r[0], f), (9, Z | C));
    // a test op without S is a no-op
    assert_eq!(
        run(imm(10, false, 1, true), &[9, 1], N | V),
        (file(&[9, 1]), N | V)
    );
}

#[test]
fn adc_sbc_rsc_use_the_carry_before_the_instruction() {
    // (opcode, result with C clear, result with C set) for r1 = 1, r2 = 2
    let table: [(u32, i32, i32); 3] = [
        (5, 1 + 2, 1 + 2 + 1),
        (6, 1 - 2 - 1, 1 - 2),
        (7, 2 - 1 - 1, 2 - 1),
    ];
    for (op, clear, set) in table {
        for (f, want) in [(0, clear), (C, set)] {
            let (r, _) = run(by_reg(op, false, 0), &[9, 1, 2, 0], f);
            assert_eq!(r[0], want as u32, "op {op} flags {f}");
        }
    }
    // a 64-bit add: adds then adc
    let (r, f) = run(shifted(4, true, 0, 0), &[9, 0xFFFF_FFFF, 1], 0);
    assert_eq!((r[0], f), (0, Z | C));
    let (r, _) = run(shifted(5, false, 0, 0), &[9, 0, 0], f);
    assert_eq!(r[0], 1);
}

#[test]
fn logic_ops_take_the_shifter_carry_and_keep_v() {
    // movs r0, r2, lsl #1 with bit 31 set: C = 1, V kept
    let (r, f) = run(shifted(13, true, 0, 1), &[9, 0, 0x8000_0001], V);
    assert_eq!((r[0], f), (2, C | V));
    // ands with a rotated immediate: C = bit 31 of the immediate, Z from the result
    let (r, f) = run(imm(0, true, 0x8000_0000, false), &[9, 0x7FFF_FFFF], C | V);
    assert_eq!((r[0], f), (0, Z | C | V));
    // the same kind of immediate with rotation 0 takes C from the flags
    assert_eq!(run(imm(0, true, 0xFF, true), &[9, 0], 0).1, Z);
    assert_eq!(run(imm(0, true, 0xFF, true), &[9, 0], C).1, Z | C);
    // teq of equal values sets Z; bic and mvn
    assert_eq!(run(imm(9, true, 7, true), &[9, 7], N).1, Z);
    assert_eq!(run(imm(14, false, 0xF0, true), &[9, 0xFF], 0).0[0], 0x0F);
    assert_eq!(run(imm(15, false, 0xF0, true), &[9, 0], 0).0[0], !0xF0);
}

#[test]
fn immediate_shift_encodings() {
    let val = 0x8000_0003;
    // lsr #0 is lsr #32, asr #0 is asr #32, ror #0 is rrx
    let (r, f) = run(shifted(13, true, 1, 0), &[9, 0, val], 0);
    assert_eq!((r[0], f), (0, Z | C));
    let (r, f) = run(shifted(13, true, 2, 0), &[9, 0, val], 0);
    assert_eq!((r[0], f), (0xFFFF_FFFF, N | C));
    let (r, f) = run(shifted(13, true, 3, 0), &[9, 0, val], C);
    assert_eq!((r[0], f), (0xC000_0001, N | C));
    let (r, f) = run(shifted(13, true, 3, 0), &[9, 0, val], 0);
    assert_eq!((r[0], f), (0x4000_0001, C));
    // lsl #0 is the plain register and keeps C
    assert_eq!(run(shifted(13, true, 0, 0), &[9, 0, 5], C).1, C);
    // ror #4
    assert_eq!(
        run(shifted(13, false, 3, 4), &[9, 0, 0x1234_5678], 0).0[0],
        0x8123_4567
    );
}

#[test]
fn register_shift_amounts_use_the_low_byte() {
    let one = 0x8000_0001;
    // (type, amount register, result, carry-out) with C = 1 before
    let table = [
        (0, 0u32, one, true),       // lsl by 0: unchanged, C kept
        (0, 1, 2, true),            // lsl 1: bit 31 out
        (0, 32, 0, true),           // lsl 32: 0, C = bit 0
        (0, 33, 0, false),          // lsl 33: 0, C = 0
        (0, 0x100, one, true),      // low byte 0: unchanged
        (0, 0xAB00_0001, 2, true),  // only the low byte counts
        (1, 32, 0, true),           // lsr 32: 0, C = bit 31
        (1, 64, 0, false),          // lsr 64
        (2, 33, 0xFFFF_FFFF, true), // asr 33: sign fill, C = bit 31
        (2, 5, 0xFC00_0000, false), // asr 5: C = bit 4 = 0
        (3, 32, one, true),         // ror 32: unchanged, C = bit 31
        (3, 33, 0xC000_0000, true), // ror 33 = ror 1: C = bit 0
        (3, 0x100, one, true),      // ror by 256: low byte 0, C kept
    ];
    for (ty, amount, res, carry) in table {
        let (r, f) = run(by_reg(13, true, ty), &[9, 0, one, amount], C);
        assert_eq!(
            (r[0], f & C != 0),
            (res, carry),
            "ty {ty} amount {amount:#x}"
        );
    }
}

#[test]
fn the_specialised_ids_do_what_their_names_say() {
    let spec = |kind, rn, rm, imm, sh| Uop {
        rn,
        rm,
        imm,
        sh,
        ..Uop::new(kind, 0x1000)
    };
    let regs = [9, 100, 30];
    assert_eq!(run(spec(k::MOV_IMM, 0, 0, 77, 0), &regs, N).0[0], 77);
    assert_eq!(run(spec(k::MOV_REG, 0, 2, 0, 0), &regs, 0).0[0], 30);
    assert_eq!(run(spec(k::MVN_IMM, 0, 0, 1, 0), &regs, 0).0[0], !1);
    assert_eq!(run(spec(k::ADD_IMM, 1, 0, 5, 0), &regs, 0).0[0], 105);
    assert_eq!(run(spec(k::SUB_IMM, 1, 0, 5, 0), &regs, 0).0[0], 95);
    assert_eq!(run(spec(k::ADD_REG, 1, 2, 0, 0), &regs, 0).0[0], 130);
    assert_eq!(run(spec(k::SUB_REG, 1, 2, 0, 0), &regs, 0).0[0], 70);
    assert_eq!(run(spec(k::ORR_IMM, 1, 0, 3, 0), &regs, 0).0[0], 103);
    // compares write only flags (and never a register)
    assert_eq!(
        run(spec(k::CMP_IMM, 1, 0, 100, 0), &regs, 0),
        (file(&regs), Z | C)
    );
    assert_eq!(run(spec(k::CMP_REG, 2, 1, 0, 0), &regs, 0).1, N);
    // tst imm: rotation 0 keeps C, a rotated immediate takes its bit 31; tst reg keeps C and V
    assert_eq!(run(spec(k::TST_IMM, 1, 0, 0x04, 1), &regs, C).1, C);
    assert_eq!(
        run(spec(k::TST_IMM, 1, 0, 0x8000_0000, 0), &[9, 0x8000_0000], 0).1,
        N | C
    );
    assert_eq!(run(spec(k::TST_REG, 1, 2, 0, 0), &regs, C | V).1, C | V);
    assert_eq!(
        run(spec(k::TST_REG, 1, 2, 0, 0), &[9, 0xF0, 0x0F], C | V).1,
        Z | C | V
    );
}

#[test]
fn a_destination_that_is_also_a_source_reads_before_it_writes() {
    // adds r1, r1, r1, lsl #1: both reads see the old r1
    let u = Uop {
        rd: 1,
        rn: 1,
        rm: 1,
        sh: 1,
        ..Uop::new(k::dp(1, 4, true), 0x1000)
    };
    assert_eq!(run(u, &[9, 5], 0).0[1], 5 + 10);
    // mov r2, r2, lsl r2
    let u = Uop {
        rd: 2,
        rm: 2,
        rs: 2,
        ..Uop::new(k::dp(2, 13, false), 0x1000)
    };
    assert_eq!(run(u, &[0, 0, 3], 0).0[2], 3 << 3);
}
