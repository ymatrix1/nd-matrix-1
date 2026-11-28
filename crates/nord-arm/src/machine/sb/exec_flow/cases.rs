//! OWNER: W2-XCORE (tests). `exec_flow` arm by arm: the `Step` each control uop produces for
//! every condition and flag nibble, the loop accumulator, `GENERIC*` against the interpreter's
//! handlers, and the rule that no other kind reaches the function.
//!
//! The suite runs unmutated as ordinary tests and whole under each mutant of `exec_flow.rs`
//! (`exec/kills.rs`): one failed assertion is a kill.

use super::*;
use crate::machine::sb::trace::testexec::suite;
use crate::machine::sb::uop::{Exit, ExitKind, Uop};
use crate::machine::SysBus;
use std::panic::{catch_unwind, AssertUnwindSafe};

const PC: u32 = 0x1000;

fn exit(kind: ExitKind, done: u16) -> Exit {
    Exit::new(
        kind,
        done,
        PC + 4 * (u32::from(done) - 1),
        PC + 4 * u32::from(done),
    )
}

/// A trace of `uops` with a table of five exits (`Direct` of 1..=5 instructions).
fn sb_of(uops: Vec<Uop>, n_guest: u16) -> Sb {
    let exits = (1..=5).map(|d| exit(ExitKind::Direct, d)).collect();
    Sb::from_parts(PC, n_guest, uops, exits, vec![(PC, 0)]).unwrap()
}

/// `exec_flow` on uop `i` of `sb` with a fresh machine whose flags are `f` (in the nibble only,
/// the `Cpu` keeps its own, so an arm that reads the wrong one shows).
fn step(sb: &Sb, i: usize, f: u8, lap: &mut Lap) -> (Step, Cpu, SysBus, u8) {
    let (mut cpu, mut bus, mut f) = (Cpu::new(), SysBus::new(), f);
    let s = exec_flow(&mut cpu, &mut bus, sb, i, &mut f, lap);
    (s, cpu, bus, f)
}

fn lap() -> Lap {
    Lap::start(100, false)
}

fn control(kind: u8, cond: u8, rs: u8, imm: u32) -> Uop {
    let mut u = Uop::new(kind, PC);
    (u.cond, u.rs, u.imm) = (cond, rs, imm);
    u
}

suite! {
    #[test]
    fn pred_runs_its_body_when_the_condition_holds_and_skips_exactly_imm_uops_otherwise() {
        let (mut ran, mut skipped) = (0, 0);
        for cond in 0..14u8 {
            for f in 0..16u8 {
                for imm in [0u32, 1, 3] {
                    let sb = sb_of(vec![control(k::PRED, cond, 0, imm)], 1);
                    let (s, ..) = step(&sb, 0, f, &mut lap());
                    // the interpreter's own condition test is the oracle
                    let mut c = Cpu::new();
                    unpack_flags(&mut c, f);
                    if c.cond(u32::from(cond)) {
                        assert_eq!(s, Step::Next(1), "cond {cond} f {f:04b}: body runs");
                        ran += 1;
                    } else {
                        assert_eq!(s, Step::Next(1 + imm as usize), "cond {cond} f {f:04b} imm {imm}");
                        skipped += 1;
                    }
                }
            }
        }
        assert!(ran > 200 && skipped > 200, "{ran} {skipped}");
        // `always` runs the body whatever the nibble
        let sb = sb_of(vec![control(k::PRED, 14, 0, 2)], 1);
        assert!((0..16).all(|f| step(&sb, 0, f, &mut lap()).0 == Step::Next(1)));
    }

    #[test]
    fn guard_leaves_through_its_exit_exactly_when_the_condition_holds() {
        let (mut left, mut stayed) = (0, 0);
        for cond in 0..14u8 {
            for f in 0..16u8 {
                let sb = sb_of(vec![control(k::GUARD, cond, 3, 0), control(k::EXIT, 0, 0, 0)], 2);
                let (s, ..) = step(&sb, 0, f, &mut lap());
                let mut c = Cpu::new();
                unpack_flags(&mut c, f);
                if c.cond(u32::from(cond)) {
                    assert_eq!(s, Step::Exit(3), "cond {cond} f {f:04b}");
                    left += 1;
                } else {
                    assert_eq!(s, Step::Next(1), "cond {cond} f {f:04b}");
                    stayed += 1;
                }
            }
        }
        assert!(left > 50 && stayed > 50, "{left} {stayed}");
    }

    #[test]
    fn guard_reg_eq_leaves_when_the_register_is_not_the_expected_return_address() {
        let mut u = Uop::new(k::GUARD_REG_EQ, PC);
        (u.rn, u.rs, u.imm) = (14, 2, 0x2004);
        let sb = sb_of(vec![u], 1);
        let run = |lr: u32| {
            let (mut cpu, mut bus, mut f, mut l) = (Cpu::new(), SysBus::new(), 0u8, lap());
            cpu.r[14] = lr;
            exec_flow(&mut cpu, &mut bus, &sb, 0, &mut f, &mut l)
        };
        assert_eq!(run(0x2004), Step::Next(1), "the inlined return holds");
        assert_eq!(run(0x2008), Step::Exit(2));
        assert_eq!(run(0), Step::Exit(2));
    }

    #[test]
    fn loop_back_re_enters_while_a_full_worst_case_lap_still_fits_and_leaves_otherwise() {
        // iteration = exit 2 (3 instructions), n_guest = 5 (a side exit of 5 exists)
        let sb = sb_of(vec![control(k::LOOP_BACK, 0, 2, 0)], 5);
        let at = |acc: u64, left: u64, watch: bool| {
            let mut l = Lap { acc, left, watch };
            let (s, ..) = step(&sb, 0, 0, &mut l);
            (s, l.acc)
        };
        // after this lap acc = 3: re-enter iff left >= 3 + 5
        assert_eq!(at(0, 8, false), (Step::Next(0), 3));
        assert_eq!(at(0, 7, false), (Step::Exit(2), 0), "the exit is applied with the old acc");
        assert_eq!(at(3, 11, false), (Step::Next(0), 6), "second lap: 6 + 5 <= 11");
        assert_eq!(at(3, 10, false), (Step::Exit(2), 3));
        assert_eq!(at(0, 1_000_000, true), (Step::Exit(2), 0), "watch: leave at the loop head");
    }

    #[test]
    fn the_exit_uops_name_their_exit_and_do_nothing_else() {
        for kind in [k::EXIT, k::EXIT_REG, k::EXIT_REG_ARM] {
            let sb = sb_of(vec![control(kind, 0, 4, 0)], 5);
            let (s, cpu, bus, f) = step(&sb, 0, 0b1010, &mut lap());
            assert_eq!((s, f, cpu.r, bus.pc), (Step::Exit(4), 0b1010, [0; 16], 0));
        }
    }

    #[test]
    fn generic_runs_a_non_ender_through_the_interpreter_handler_and_reloads_the_nibble() {
        // subs r0,r0,#1 : the result 0 sets Z and C in the nibble, which the next uop reads
        let sb = sb_of(vec![Uop::generic(0xE250_0001, PC, 1)], 1);
        let (mut cpu, mut bus, mut f) = (Cpu::new(), SysBus::new(), 0b1001u8);
        cpu.r[0] = 1;
        let s = exec_flow(&mut cpu, &mut bus, &sb, 0, &mut f, &mut lap());
        assert_eq!((s, cpu.r[0], f, bus.pc), (Step::Next(1), 0, 0b0110, PC));
        // a conditional instruction reads the nibble, not the stale flags of the cpu: addne is
        // skipped with Z in the nibble although the cpu says "not zero"
        let sb = sb_of(vec![Uop::generic(0x1280_0001, PC, 1)], 1);
        let (mut cpu, mut f) = (Cpu::new(), 0b0100u8);
        exec_flow(&mut cpu, &mut SysBus::new(), &sb, 0, &mut f, &mut lap());
        assert_eq!(cpu.r[0], 0, "ne does not hold: Z is set in the nibble");
    }

    #[test]
    fn generic_leaves_through_its_early_exit_only_when_a_store_made_code_stale() {
        let sb = sb_of(vec![Uop::generic(0xE582_1000, PC, 3)], 1); // str r1,[r2]
        let mut bus = SysBus::new();
        bus.mem.write32(0x1000, 0);
        bus.mem.write32(0x9000_0000, 0);
        bus.mem.mark_code(0x1000);
        let (mut cpu, mut f) = (Cpu::new(), 0u8);
        (cpu.r[1], cpu.r[2]) = (0x9000_0000, 0x9000_0000);
        // a store to a data page: no exit
        let s = exec_flow(&mut cpu, &mut bus, &sb, 0, &mut f, &mut lap());
        assert_eq!((s, bus.mem.stale_code.is_empty()), (Step::Next(1), true));
        // a store into the code page: the early exit (`cond` of the uop)
        cpu.r[2] = 0x1010;
        let s = exec_flow(&mut cpu, &mut bus, &sb, 0, &mut f, &mut lap());
        assert_eq!((s, bus.mem.stale_code.clone()), (Step::Exit(3), vec![1]));
    }

    #[test]
    fn generic_end_spills_the_nibble_runs_the_ender_and_leaves_through_its_exit() {
        // beq .+0x10 with Z only in the nibble: taken, the target is left in r15 (Dynamic)
        let word = 0x0A00_0002; // beq pc+8+8
        let sb = sb_of(vec![Uop::generic_end(word, PC, 1)], 2);
        let (s, cpu, bus, f) = step(&sb, 0, 0b0100, &mut lap());
        assert_eq!((s, cpu.r[15], f, bus.pc), (Step::Exit(1), PC + 16, 0b0100, PC));
        // not taken: the fall-through
        let (s, cpu, ..) = step(&sb, 0, 0b0000, &mut lap());
        assert_eq!((s, cpu.r[15]), (Step::Exit(1), PC + 4));
        // a fault is left for the driver and the exit is still taken
        let sb = sb_of(vec![Uop::generic_end(0xE7F0_00F0, PC, 0)], 1);
        let (s, cpu, ..) = step(&sb, 0, 0, &mut lap());
        assert_eq!((s, cpu.fault.is_some(), cpu.r[15]), (Step::Exit(0), true, PC + 4));
    }

    #[test]
    fn mrs_and_msr_continue_with_the_next_uop() {
        let mut m = Uop::new(k::MRS, PC);
        m.rd = 5;
        let mut w = Uop::new(k::MSR_C, PC);
        (w.rs, w.imm) = (1, 0x1F);
        let sb = sb_of(vec![m, w], 2);
        let (s, cpu, _, f) = step(&sb, 0, 0b1000, &mut lap());
        assert_eq!((s, f), (Step::Next(1), 0b1000));
        assert_eq!(cpu.r[5] >> 28, 0b1000, "flags from the nibble");
        let (s, cpu, ..) = step(&sb, 1, 0, &mut lap());
        assert_eq!((s, cpu.mode, cpu.mode_changes), (Step::Next(2), 0x1F, 1));
    }

    #[test]
    fn the_fused_uops_continue_or_leave_through_the_exit_of_their_guard() {
        let mut g = Uop::new(k::CMP_GUARD_I, PC);
        (g.rn, g.imm, g.cond, g.rs, g.n) = (1, 7, 0, 3, 2); // cmp r1,#7 ; beq -> exit 3
        let mut mp = Uop::new(k::MOV_PAIR, PC);
        (mp.rd, mp.rn, mp.imm) = (2, 3, 5 | 6 << 16);
        let sb = sb_of(vec![g, mp], 4);
        for (r1, want) in [(7u32, Step::Exit(3)), (8, Step::Next(1))] {
            let (mut cpu, mut bus, mut f) = (Cpu::new(), SysBus::new(), 0u8);
            cpu.r[1] = r1;
            let s = exec_flow(&mut cpu, &mut bus, &sb, 0, &mut f, &mut lap());
            assert_eq!((s, f & 4 != 0), (want, r1 == 7), "the compare wrote Z");
        }
        let (s, cpu, ..) = step(&sb, 1, 0, &mut lap());
        assert_eq!((s, cpu.r[2], cpu.r[3]), (Step::Next(2), 5, 6));
    }

    #[test]
    fn a_kind_that_is_not_a_control_or_fused_uop_never_reaches_the_function() {
        for kind in [k::MOV_IMM, k::LDR_IMM, 0, 108, 143, 255] {
            let sb = sb_of(vec![Uop::new(kind, PC)], 1);
            let r = catch_unwind(AssertUnwindSafe(|| step(&sb, 0, 0, &mut lap())));
            assert!(r.is_err(), "kind {kind}");
        }
    }
}
