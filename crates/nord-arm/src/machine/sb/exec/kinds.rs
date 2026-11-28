//! OWNER: W2-XCORE (tests). Layer L6: every defined kind id has an arm. One uop of each of the
//! 145 allocated ids runs through `run_sb` and none reaches an `unreachable!`; every other id
//! (the gaps `129..=143`, `155..=159` and `165..=255`) does, by design.

use super::*;
use crate::machine::sb::exec_mem::{F_P, F_U, F_WB};
use crate::machine::sb::uop::Uop;
use crate::machine::SysBus;
use std::panic::{catch_unwind, AssertUnwindSafe};

const PC: u32 = 0x1000;
const DATA: u32 = 0x9000_0000;

/// Exits of the sample traces: 0 is the terminal `EXIT`, 1 a guard or early exit, 2 an `Indirect`
/// exit through `r4`, 3 an `IndirectArm` exit through `r4`, 4 the `LoopBack`.
fn exits() -> Vec<Exit> {
    let mut indirect = Exit::new(ExitKind::Indirect, 1, PC, 0);
    indirect.reg = 4;
    let mut arm = Exit::new(ExitKind::IndirectArm, 1, PC, 0);
    arm.reg = 4;
    vec![
        Exit::new(ExitKind::Direct, 2, PC + 4, 0x2000),
        Exit::new(ExitKind::Stale, 1, PC, PC + 4),
        indirect,
        arm,
        Exit::new(ExitKind::LoopBack, 1, PC, PC),
    ]
}

/// A uop of `kind` with operands that make it safe and meaningful to run on [`machine`], and
/// the index of the exit it is expected to leave through when it ends the trace by itself.
fn sample(kind: u8) -> (Uop, Option<usize>) {
    let mut u = Uop::new(kind, PC);
    match k::family(kind) {
        k::Family::Dp => {
            (u.rd, u.rn, u.rm, u.rs) = (2, 1, 3, 3);
            u.imm = 7;
        }
        k::Family::Mem => {
            (u.rd, u.rn, u.rm, u.rs, u.imm) = (2, 1, 3, F_U | F_P | F_WB, 4);
            if matches!(kind, k::LDM | k::STM) {
                (u.sh, u.imm) = (2, 0b1100);
            }
            if kind == k::LDR_ABS {
                u.imm = DATA;
            }
        }
        _ => {}
    }
    let mut exit = None;
    match kind {
        k::PRED => (u.cond, u.imm) = (14, 0),
        k::GUARD => (u.cond, u.rs) = (0, 1), // EQ with the flags clear: not taken
        k::GUARD_REG_EQ => (u.rn, u.imm, u.rs) = (0, 0, 1), // r0 == 0: not taken
        k::LOOP_BACK => (u.rs, exit) = (4, Some(4)),
        k::EXIT => exit = Some(0),
        k::EXIT_REG => (u.rn, u.rs, exit) = (4, 2, Some(2)),
        k::EXIT_REG_ARM => (u.rn, u.rs, exit) = (4, 3, Some(3)),
        k::GENERIC => u = Uop::generic(0xE1A0_0000, PC, 1), // mov r0,r0
        k::GENERIC_END => {
            u = Uop::generic_end(0xEAFF_FFFE, PC, 0); // b .
            exit = Some(0);
        }
        k::MRS => u.rd = 2,
        k::MSR_C => (u.rs, u.imm) = (1, 0x13),
        k::CMP_GUARD_I | k::CMP_GUARD_R | k::TST_GUARD_I => {
            (u.rn, u.rm, u.imm, u.cond, u.rs, u.n) = (1, 3, 99, 0, 1, 2);
        }
        k::MOV_PAIR => (u.rd, u.rn, u.imm, u.n) = (2, 3, 5 | 6 << 16, 2),
        k::LDR_PAIR => {
            (u.rd, u.rm, u.rn, u.rs, u.imm, u.n) = (2, 3, 1, F_U | F_P, 4 << 16, 2);
        }
        _ => {}
    }
    (u, exit)
}

fn machine() -> (Cpu, SysBus) {
    let (mut cpu, mut bus) = (Cpu::new(), SysBus::new());
    bus.mem.write32(DATA, 0x1111_1111);
    bus.mem.write32(DATA + 4, 0x2222_2222);
    bus.mem.write32(DATA + 8, 0x3333_3333);
    cpu.r[1] = DATA;
    cpu.r[3] = 4;
    cpu.r[4] = 0x6001;
    (cpu, bus)
}

#[test]
fn every_defined_kind_id_runs_through_its_arm() {
    let mut ran = 0;
    for kind in 0..=255u8 {
        if k::family(kind) == k::Family::Invalid {
            continue;
        }
        let (u, exit) = sample(kind);
        let uops = vec![u, Uop::exit(0, PC + 4)];
        let sb = Sb::from_parts(PC, 4, uops, exits(), vec![(PC, 0)]).unwrap();
        let (mut cpu, mut bus) = machine();
        let mut left = 4u64;
        let out = run_sb(&mut cpu, &mut bus, &sb, &mut left, false);
        let e = &sb.exits[out.exit as usize];
        assert_eq!(
            (cpu.insns, left),
            (u64::from(e.done), 4 - u64::from(e.done)),
            "kind {kind}"
        );
        if let Some(want) = exit {
            assert_eq!(usize::from(out.exit), want, "kind {kind} ends the trace");
        }
        ran += 1;
    }
    assert_eq!(ran, k::DEFINED);
}

#[test]
fn an_id_that_is_not_allocated_is_unreachable_not_silently_skipped() {
    let sb_of = |kind: u8| {
        let uops = vec![Uop::new(kind, PC), Uop::exit(0, PC)];
        Sb::from_parts(PC, 2, uops, exits(), vec![(PC, 0)]).unwrap()
    };
    let mut checked = 0;
    for kind in (0..=255u8).filter(|k| k::family(*k) == k::Family::Invalid) {
        let sb = sb_of(kind);
        let r = catch_unwind(AssertUnwindSafe(|| {
            let (mut cpu, mut bus) = machine();
            run_sb(&mut cpu, &mut bus, &sb, &mut 4, false)
        }));
        assert!(r.is_err(), "kind {kind} must not run");
        checked += 1;
    }
    assert_eq!(checked, 256 - k::DEFINED);
}

#[test]
fn a_trace_that_runs_off_its_last_uop_is_a_bug_of_the_builder_and_says_so() {
    let sb = Sb::from_parts(
        PC,
        1,
        vec![Uop::new(k::MOV_IMM, PC)],
        exits(),
        vec![(PC, 0)],
    )
    .unwrap();
    let r = catch_unwind(AssertUnwindSafe(|| {
        let (mut cpu, mut bus) = machine();
        run_sb(&mut cpu, &mut bus, &sb, &mut 4, false)
    }));
    let payload = r.unwrap_err();
    let msg = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(msg.contains("terminal uop"), "{msg}");
}
