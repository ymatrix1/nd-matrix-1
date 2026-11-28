//! OWNER: W2-DRV (tests). The watch contract with W3-LOOP, decided with the interpreter as the
//! oracle: with the watch on, the executor leaves a `LOOP_BACK` through its own exit of kind
//! `ExitKind::LoopBack` (target: the loop head), no builder ever emits `ExitKind::Watch`, and the
//! driver turns the dispatch at `watch.pc` into the watch return.

use super::cycle_tests::{counting, machine, state, CODE};
use super::{run_fast, run_sb_full};
use crate::machine::sb::exec::run_sb;
use crate::machine::sb::testgen::asm::{self, AL};
use crate::machine::sb::testgen::random_program;
use crate::machine::sb::testkit::{compare_machines, Rng};
use crate::machine::sb::trace::{build, BuildOpts};
use crate::machine::sb::uop::{k, ExitKind};
use crate::machine::sb::{SbConfig, Watch};
use crate::machine::Stop;

/// `add r0` ; `add r1` ; `b CODE`: a loop of three instructions.
fn spin3() -> Vec<u32> {
    vec![
        asm::add_imm(false, 0, 0, 1),
        asm::add_imm(false, 1, 1, 2),
        asm::branch(AL, false, CODE + 8, CODE),
    ]
}

#[test]
fn a_loop_back_leaves_through_its_own_loop_back_exit_when_watched() {
    let words = spin3();
    let mut m = machine(&words);
    let sb = build(&mut m.bus.mem, CODE, &BuildOpts::default()).unwrap();
    assert!(sb.uops.iter().any(|u| u.kind == k::LOOP_BACK));
    for (watch, budget) in [(true, 100u64), (false, 100)] {
        let mut m = machine(&words);
        let mut left = budget;
        let out = run_sb(&mut m.cpu, &mut m.bus, &sb, &mut left, watch);
        let e = &sb.exits[usize::from(out.exit)];
        assert_eq!(
            (e.kind, out.target),
            (ExitKind::LoopBack, CODE),
            "watch {watch}"
        );
        // the interpreter, run for as many instructions as the trace retired, is the oracle
        let mut want = machine(&words);
        want.predecode = false;
        let n = m.cpu.insns;
        let w = want.run(n, 0, None);
        compare_machines(&want, w, &m, (n, Stop::Budget)).unwrap();
        assert_eq!(
            n,
            if watch { 3 } else { 99 },
            "one lap when watched, else all that fit"
        );
    }
}

#[test]
fn no_builder_emits_a_watch_exit() {
    let (mut loops, mut traces) = (0, 0);
    for seed in 0..40u64 {
        let words = random_program(&mut Rng::new(seed), 64);
        let mut m = machine(&words);
        for i in 0..words.len() as u32 {
            let opts = BuildOpts::from(&SbConfig::on());
            let Some(sb) = build(&mut m.bus.mem, CODE + 4 * i, &opts) else {
                continue;
            };
            traces += 1;
            loops += sb.uops.iter().filter(|u| u.kind == k::LOOP_BACK).count();
            assert!(sb.exits.iter().all(|e| e.kind != ExitKind::Watch));
        }
    }
    assert!(
        traces > 1000 && loops >= 5,
        "exercised: {traces} traces, {loops} loops"
    );
}

#[test]
fn an_armed_watch_returns_at_the_loop_head_after_each_lap_with_the_interpreter_state() {
    let words = spin3();
    let mut got = machine(&words);
    let mut sb = state(SbConfig::on());
    sb.watch = Watch {
        armed: true,
        hit: false,
        pc: CODE,
    };
    for lap in 1..=6u64 {
        assert_eq!(run_fast(&mut got, &mut sb, 1000, 0, u32::MAX), None);
        assert!(sb.watch.hit, "lap {lap}");
        sb.watch.hit = false;
        let mut want = machine(&words);
        want.predecode = false;
        let w = want.run(3 * lap, 0, None);
        compare_machines(&want, w, &got, (3 * lap, Stop::Budget)).unwrap();
    }
    assert!(
        sb.stats.enters >= 6,
        "the laps ran in the trace: {:?}",
        sb.stats
    );
}

#[test]
fn a_watch_pc_that_is_not_a_dispatch_point_never_hits_and_a_disarmed_watch_is_inert() {
    let words = counting(300);
    for armed in [true, false] {
        let (mut a, mut b) = (machine(&words), machine(&words));
        let mut sb = state(SbConfig::on());
        sb.watch = Watch {
            armed,
            hit: false,
            pc: CODE + 8, // the middle of the loop body: the traces never return there
        };
        a.predecode = false;
        let w = a.run(200, 0, None);
        let g = run_sb_full(&mut b, &mut sb, 200, 0, None);
        compare_machines(&a, w, &b, g).unwrap();
        assert!(!sb.watch.hit, "armed {armed}");
        assert!(sb.stats.enters > 0);
    }
}

#[test]
fn run_sb_full_ends_a_call_at_the_watch_hit_and_the_next_call_goes_on() {
    let words = spin3();
    let mut m = machine(&words);
    let mut sb = state(SbConfig::on());
    sb.watch = Watch {
        armed: true,
        hit: false,
        pc: CODE,
    };
    assert_eq!(
        run_sb_full(&mut m, &mut sb, 100, 0, None),
        (3, Stop::Budget)
    );
    assert!(sb.watch.hit);
    sb.watch.armed = false;
    sb.watch.hit = false;
    assert_eq!(
        run_sb_full(&mut m, &mut sb, 97, 0, None),
        (97, Stop::Budget)
    );
    assert_eq!(m.cpu.insns, 100);
}
