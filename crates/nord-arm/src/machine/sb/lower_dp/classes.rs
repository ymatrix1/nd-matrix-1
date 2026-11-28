//! Class-level tests of `lower_dp`: 300 random words per predecode class against a decision
//! model written from the rules of the design note, the folded constants against the
//! interpreter, and a check that every mutant site is live. `cfg(test)` only.
//!
//! These are decision tests: they say which uop is chosen and with which fields. That the uop
//! computes what the interpreter computes is the business of `exec_alu` (W1-XALU) and the L0
//! layer of `difftest` (W3-VERIF); the one semantic check here is the value of a folded pc.

use super::support::{class_word, dp, imm_word, lower, reg_word, rsr_word, some_pc, AL};
use super::{Lowered, MUTANT_SITES};
use crate::cpu::predecode::op;
use crate::cpu::Cpu;
use crate::machine::sb::decode::{decode, touches_r15};
use crate::machine::sb::mutants;
use crate::machine::sb::testkit::{random_cpu, LogBus, Rng};
use crate::machine::sb::uop::{k, Uop};

/// `(opcode, s, form, specialised kind)`: the shapes the design note gives a specialised id (a
/// register operand only with `lsl #0`; compares and tests are always S).
const SPECIALISED: [(u32, bool, u32, u8); 12] = [
    (13, false, 0, k::MOV_IMM),
    (13, false, 1, k::MOV_REG),
    (15, false, 0, k::MVN_IMM),
    (4, false, 0, k::ADD_IMM),
    (4, false, 1, k::ADD_REG),
    (2, false, 0, k::SUB_IMM),
    (2, false, 1, k::SUB_REG),
    (10, true, 0, k::CMP_IMM),
    (10, true, 1, k::CMP_REG),
    (8, true, 0, k::TST_IMM),
    (8, true, 1, k::TST_REG),
    (12, false, 0, k::ORR_IMM),
];

/// What the design note says `lower_dp` does with `word` at `pc`, derived from the bits and the
/// table above rather than from the implementation. `None` is `GENERIC`; the flag is true for a
/// folded pc.
fn model(word: u32, pc: u32) -> Option<(Uop, bool)> {
    let form = match (word >> 25 & 1, word >> 4 & 1) {
        (1, _) => 0,
        (_, 1) => 2,
        _ => 1,
    };
    let (opcode, s) = (word >> 21 & 15, word >> 20 & 1 == 1);
    let reg = |shift: u32| (word >> shift & 15) as u8;
    let (rn, rd, rm, rs) = (reg(16), reg(12), reg(0), reg(8));
    let test = (8..=11).contains(&opcode);
    let reads_rn = opcode != 13 && opcode != 15;
    let rot = word >> 8 & 15;
    let (imm, sh) = match form {
        0 => ((word & 0xFF).rotate_right(2 * rot), u8::from(rot == 0)),
        1 => (0, ((word >> 5 & 3) << 5 | word >> 7 & 31) as u8),
        _ => (0, ((word >> 5 & 3) << 5) as u8),
    };
    if !test && rd == 15 {
        return None;
    }
    let mut u = Uop::new(k::MOV_IMM, pc);
    u.rd = rd;
    if form == 0 && opcode == 4 && !s && rn == 15 {
        u.imm = pc.wrapping_add(8).wrapping_add(imm);
        return Some((u, true));
    }
    if form == 1 && opcode == 13 && !s && sh == 0 && rm == 15 {
        u.imm = pc.wrapping_add(8);
        return Some((u, true));
    }
    if (reads_rn && rn == 15) || (form > 0 && rm == 15) || (form == 2 && rs == 15) {
        return None;
    }
    let plain = form == 0 || (form == 1 && sh == 0);
    let spec = SPECIALISED
        .iter()
        .find(|(o, ss, f, _)| (*o, *ss, *f) == (opcode, s, form))
        .filter(|_| plain);
    u.kind = spec.map_or(k::dp(form, opcode, s), |e| e.3);
    u.rd = if test { 0 } else { rd };
    u.rn = if reads_rn { rn } else { 0 };
    u.rm = if form > 0 { rm } else { 0 };
    u.rs = if form == 2 { rs } else { 0 };
    u.imm = imm;
    u.sh = if spec.is_some() && u.kind != k::TST_IMM {
        0
    } else {
        sh
    };
    Some((u, false))
}

/// 300 random words of every one of the 96 generic ids against [`model`]; panics on the first
/// disagreement. A function, not a test, so the mutant test can run it under each mutant.
fn check_every_class() {
    let mut rng = Rng::new(0xD1CE);
    let mut by_kind = [0u32; k::COUNT];
    let (mut folds, mut rejected) = (0, 0);
    for id in 0..96u8 {
        let (form, opcode, s) = (u32::from(id >> 5), u32::from(id >> 1 & 15), id & 1 != 0);
        let reachable = !(8..=11).contains(&opcode) || s;
        let (mut done, mut generic) = (0, 0);
        for _ in 0..300 {
            let (word, pc) = (class_word(&mut rng, id), some_pc(&mut rng));
            let (lowered, out) = lower(word, pc);
            let want = if reachable { model(word, pc) } else { None };
            assert_eq!(
                decode(pc, word).entry.op == op::DP_BASE + id,
                reachable,
                "{word:#010x}"
            );
            match want {
                Some((u, folded)) => {
                    let msg = format!("{word:#010x} at {pc:#x}");
                    assert_eq!((lowered, &out[..]), (Lowered::Done, &[u][..]), "{msg}");
                    assert!(!touches_r15(&u), "I2: {u:?}");
                    assert_eq!((u.n, u.cond, u.pc), (1, 0, pc), "{msg}");
                    by_kind[u.kind as usize] += 1;
                    folds += u32::from(folded);
                    done += 1;
                }
                None => {
                    assert_eq!((lowered, out.len()), (Lowered::Generic, 0), "{word:#010x}");
                    generic += 1;
                }
            }
        }
        rejected += generic;
        // every reachable class is exercised both ways, so neither side of the test is vacuous
        assert_eq!(done + generic, 300, "id {id} form {form}");
        assert_eq!(
            done > 0 && generic > 0,
            reachable,
            "id {id}: done {done}, generic {generic}"
        );
        // a class without a form-0 specialised id also shows up under its own generic id
        let shadowed = form == 0
            && SPECIALISED
                .iter()
                .any(|e| (e.0, e.1, e.2) == (opcode, s, 0));
        assert!(
            !reachable || shadowed || by_kind[id as usize] > 0,
            "generic id {id} never produced"
        );
    }
    for (_, _, _, kind) in SPECIALISED {
        assert!(
            by_kind[kind as usize] >= 20,
            "specialised kind {kind}: {}",
            by_kind[kind as usize]
        );
    }
    assert!(
        folds >= 40 && rejected > 3000,
        "folds {folds}, rejected {rejected}"
    );
}

#[test]
fn random_words_of_every_class_follow_the_decision_model() {
    check_every_class();
}

#[test]
fn the_decision_model_kills_every_mutant_site() {
    for (id, name) in MUTANT_SITES {
        let killed = std::panic::catch_unwind(|| mutants::with(*id, check_every_class)).is_err();
        assert!(killed, "mutant {id} {name} survives the class test");
    }
}

#[test]
fn lowering_appends_after_what_is_already_in_out() {
    let sentinel = Uop::new(k::MOV_IMM, 0x0FFC);
    let opts = crate::machine::sb::trace::BuildOpts::default();
    let mut out = vec![sentinel];
    let mut fold = |_: u32| None;
    let mut cx = crate::machine::sb::decode::LowerCtx {
        insn: decode(0x1000, 0xE280_0001),
        out: &mut out,
        fold: &mut fold,
        opts: &opts,
    };
    assert_eq!(super::lower_dp(&mut cx), Lowered::Done);
    assert_eq!((out.len(), out[0], out[1].kind), (2, sentinel, k::ADD_IMM));
}

#[test]
fn folded_constants_equal_what_the_interpreter_writes() {
    let mut rng = Rng::new(7);
    let (mut adds, mut movs) = (0, 0);
    for case in 0..600 {
        let (pc, rd) = (some_pc(&mut rng), rng.below(15));
        let word = if case % 2 == 0 {
            adds += 1; // add rd,pc,#x
            imm_word(dp(AL, 4, false, 15, rd), rng.below(16), rng.below(256))
        } else {
            movs += 1; // mov rd,pc (Rn is SBZ and ignored, whatever it holds)
            reg_word(dp(AL, 13, false, rng.below(16), rd), 0, 0, 15)
        };
        let (lowered, out) = lower(word, pc);
        assert_eq!(
            (lowered, out.len(), out[0].kind),
            (Lowered::Done, 1, k::MOV_IMM)
        );
        let mut bus = LogBus::new(1);
        bus.poke32(pc, word);
        let mut cpu = random_cpu(&mut rng, pc);
        let ran = cpu.clone();
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[rd as usize], out[0].imm, "{word:#010x} at {pc:#x}");
        assert_eq!(cpu.r[15], pc.wrapping_add(4));
        for i in (0..15).filter(|i| *i != rd as usize) {
            assert_eq!(cpu.r[i], ran.r[i], "r{i} is not written");
        }
        assert_eq!(cpu.nzcv(), ran.nzcv(), "a non-S fold leaves the flags");
        assert_eq!(out[0].rd, rd as u8);
    }
    assert_eq!((adds, movs), (300, 300));
}

#[test]
fn a_register_shifted_pc_reads_twelve_ahead_so_form_2_has_no_fold() {
    let mut bus = LogBus::new(2);
    let head = dp(AL, 13, false, 0, 0); // mov r0, ...
    let (shifted, plain) = (rsr_word(head, 1, 0, 15), reg_word(head, 0, 0, 15));
    bus.poke32(0x2000, shifted); // mov r0,pc,lsl r1 with r1 = 0
    bus.poke32(0x3000, plain); // mov r0,pc
    let mut cpu = Cpu::new();
    cpu.r[15] = 0x2000;
    cpu.step(&mut bus).unwrap();
    assert_eq!(cpu.r[0], 0x2000 + 12, "form 2 reads pc + 12");
    cpu.r[15] = 0x3000;
    cpu.step(&mut bus).unwrap();
    assert_eq!(cpu.r[0], 0x3000 + 8, "form 1 reads pc + 8");
    assert_eq!(lower(shifted, 0x2000).0, Lowered::Generic);
    assert_eq!(lower(plain, 0x3000).1[0].imm, 0x3000 + 8);
}

#[test]
fn every_mutant_site_changes_a_decision() {
    let probes: [u32; 11] = [
        0xE315_0010,                               // 0: tst r5,#0x10 (rot == 0)
        0xE315_020F,                               // 1: tst r5,#0xF0000000 (rot != 0)
        0xE3A0_34FF,                               // 2: mov r3,#0xFF000000
        0xE28F_0004,                               // 3: add r0,pc,#4
        reg_word(dp(AL, 0, false, 1, 0), 3, 1, 2), // 4: and r0,r1,r2,lsr #3
        0xE081_0312,                               // 5: add r0,r1,r2,lsl r3
        0xE241_0004,                               // 6: sub r0,r1,#4
        0xE151_0002,                               // 7: cmp r1,r2
        0xE1B0_0001,                               // 8: movs r0,r1
        0xE1A0_010F,                               // 9: mov r0,pc,lsl #2
        0xE29F_0004,                               // 10: adds r0,pc,#4
    ];
    assert_eq!(MUTANT_SITES.len(), probes.len());
    for ((id, name), word) in MUTANT_SITES.iter().zip(probes) {
        assert!(*id < 100, "{name}: outside the block reserved for W1-DP");
        let base = lower(word, 0x2000);
        let mutated = mutants::with(*id, || lower(word, 0x2000));
        assert_ne!(
            base, mutated,
            "mutant {id} {name} does not change {word:#010x}"
        );
        assert_eq!(lower(word, 0x2000), base, "the mutant switch is restored");
    }
    assert_eq!(mutants::active(), None);
}
