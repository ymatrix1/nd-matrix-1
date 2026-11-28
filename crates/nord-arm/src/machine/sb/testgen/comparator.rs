//! OWNER: W1-VGEN. Self-tests of the L0 comparator ([`compare_outcome`] and `uops_vs_step`): every
//! part of an outcome it claims to compare is perturbed on its own and must be named. The
//! comparison is the oracle of the lowering and executor work packages (I5: memory effects in
//! guest order, `bus.pc` of the accessing instruction), so a check that silently stopped
//! comparing the access log, the `pc` of an access, `bus.pc` or the retired count would blind
//! every one of their L0 tests.

use super::{asm, compare_outcome, lower_single, uops_vs_step};
use crate::cpu::{Cpu, Fault};
use crate::machine::sb::testkit::logbus::kind;
use crate::machine::sb::testkit::oracle::oracle_trace_on;
use crate::machine::sb::testkit::{random_cpu, LogBus, Rng, TraceRun};
use crate::machine::sb::uop::{Exit, ExitKind, Sb, Uop};

/// Address the one instruction under test is at.
const PC: u32 = 0x1000;

/// A CPU at [`PC`] with three distinct data pointers in `r1..r3` and distinct values in `r4`/`r5`.
fn cpu() -> Cpu {
    let mut c = random_cpu(&mut Rng::new(7), PC);
    (c.r[1], c.r[2], c.r[3]) = (0x4000, 0x5000, 0x6000);
    (c.r[4], c.r[5]) = (0x1111, 0x2222);
    c
}

/// The oracle's run of the one instruction `word` at [`PC`].
fn oracle(word: u32, cpu: &Cpu) -> TraceRun {
    let mut bus = LogBus::new(1);
    bus.poke32(PC, word);
    oracle_trace_on(cpu, bus, 1)
}

/// What an executor leaves behind: CPU, fault, bus and the instructions left of its budget of 4.
struct Outcome {
    cpu: Cpu,
    fault: Option<Fault>,
    bus: LogBus,
    left: u64,
}

impl Outcome {
    /// A faithful copy of the oracle's result: what a correct executor leaves.
    fn of(t: &TraceRun) -> Self {
        Self {
            cpu: t.cpu.clone(),
            fault: t.fault,
            bus: t.bus.clone(),
            left: 3,
        }
    }

    fn compare(&self, word: u32, t: &TraceRun) -> Result<(), String> {
        compare_outcome(word, t, &self.cpu, self.fault, &self.bus, self.left)
    }
}

/// `stmia r1, {r4, r5}`: two stores in guest order at two addresses, with different values.
fn two_stores() -> u32 {
    asm::block(asm::AL, false, false, true, false, 1, 0b11_0000)
}

#[test]
fn every_part_of_an_outcome_is_compared_and_named_on_its_own() {
    type Perturb = fn(&mut Outcome);
    let word = two_stores();
    let t = oracle(word, &cpu());
    assert_eq!(
        t.bus.log,
        vec![
            (kind::STORE32, 0x4000, 0x1111, PC),
            (kind::STORE32, 0x4004, 0x2222, PC)
        ],
        "the instruction under test stores twice, in order"
    );
    Outcome::of(&t).compare(word, &t).expect("the copy equals");
    let cases: &[(&str, &str, Perturb)] = &[
        ("a register", "r3: ", |o| o.cpu.r[3] ^= 1),
        ("a flag", "nzcv: ", |o| o.cpu.z = !o.cpu.z),
        ("the retired count", "insns: ", |o| o.cpu.insns += 1),
        ("the fault", "fault: ", |o| o.fault = Some(Fault::Wfi)),
        ("the address of an access", "access 0: st32 [0x4000]", |o| {
            o.bus.log[0].1 ^= 0x10
        }),
        (
            "the value of an access",
            "access 1: st32 [0x4004] = 0x2222",
            |o| o.bus.log[1].2 ^= 1,
        ),
        ("the pc of an access", "at pc 0x1000", |o| {
            o.bus.log[0].3 ^= 4
        }),
        ("the kind of an access", "access 0: st32", |o| {
            o.bus.log[0].0 = kind::LOAD32
        }),
        (
            "the order of two accesses",
            "access 0: st32 [0x4000]",
            |o| o.bus.log.swap(0, 1),
        ),
        ("a missing access", "extra access on the left", |o| {
            o.bus.log.pop();
        }),
        ("an extra access", "extra access on the right", |o| {
            o.bus.log.push((kind::LOAD8, 0x10, 0, PC))
        }),
        ("bus.pc", "bus.pc 0x1000 vs 0x1004", |o| o.bus.pc += 4),
        ("instructions left, too many", "left 4", |o| o.left = 4),
        ("instructions left, too few", "left 2", |o| o.left = 2),
    ];
    for (what, expect, perturb) in cases {
        let mut o = Outcome::of(&t);
        perturb(&mut o);
        let err = o
            .compare(word, &t)
            .expect_err(&format!("{what} is not compared"));
        assert!(err.contains(expect), "{what}: {err}");
    }
}

#[test]
fn a_perturbation_of_the_log_alone_leaves_the_state_comparison_clean() {
    // The cases above are only evidence for the log and `bus.pc` checks if the CPU half passes
    // before them: the error of a log-only change must come from the log check, not the CPU one.
    let word = two_stores();
    let t = oracle(word, &cpu());
    let mut o = Outcome::of(&t);
    o.bus.log[0].3 ^= 4;
    let err = o.compare(word, &t).unwrap_err();
    assert!(
        err.contains("access logs differ") && !err.contains("nzcv") && !err.contains("r1:"),
        "{err}"
    );
    let mut o = Outcome::of(&t);
    o.bus.pc += 4;
    let err = o.compare(word, &t).unwrap_err();
    assert!(
        !err.contains("access logs differ") && err.contains("bus.pc"),
        "{err}"
    );
}

#[test]
fn a_store_to_another_address_with_equal_registers_is_caught_through_the_whole_helper() {
    let (cpu, right, wrong) = (cpu(), asm::str(4, 1, 0), asm::str(4, 2, 0));
    let sb = |w| lower_single(w, PC, &|_| 0);
    uops_vs_step(right, &sb(right), &cpu, 1).expect("the lowering of the same word agrees");
    let err = uops_vs_step(right, &sb(wrong), &cpu, 1).unwrap_err();
    assert!(
        err.contains("access logs differ") && err.contains("[0x4000]") && err.contains("[0x5000]"),
        "{err}"
    );
    assert!(!err.contains("r4:"), "the registers are equal: {err}");
}

#[test]
fn a_trace_whose_exit_names_another_last_pc_is_caught_on_bus_pc_alone() {
    let (cpu, word) = (cpu(), asm::mov_imm(0, 5));
    let sb = |last_pc| {
        Sb::from_parts(
            PC,
            1,
            vec![Uop::generic(word, PC, 0), Uop::exit(0, PC)],
            vec![Exit::new(ExitKind::Direct, 1, last_pc, PC + 4)],
            vec![(PC, word)],
        )
        .unwrap()
    };
    uops_vs_step(word, &sb(PC), &cpu, 1).expect("the right last pc agrees");
    let err = uops_vs_step(word, &sb(PC + 0x40), &cpu, 1).unwrap_err();
    assert!(err.contains("bus.pc 0x1000 vs 0x1040"), "{err}");
}

#[test]
fn a_store_attributed_to_another_instruction_is_caught_on_the_pc_of_the_access() {
    let (cpu, word) = (cpu(), asm::str(4, 1, 0));
    let sb = |at| {
        Sb::from_parts(
            PC,
            1,
            vec![Uop::generic(word, at, 0), Uop::exit(0, PC)],
            vec![Exit::new(ExitKind::Direct, 1, PC, PC + 4)],
            vec![(PC, word)],
        )
        .unwrap()
    };
    uops_vs_step(word, &sb(PC), &cpu, 1).expect("the right pc agrees");
    let err = uops_vs_step(word, &sb(PC + 0x20), &cpu, 1).unwrap_err();
    assert!(
        err.contains("access logs differ")
            && err.contains("at pc 0x1000")
            && err.contains("0x1020"),
        "{err}"
    );
}
