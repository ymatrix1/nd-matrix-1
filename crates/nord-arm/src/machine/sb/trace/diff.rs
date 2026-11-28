//! OWNER: W1-FLOW (tests). Differential tests of trace formation: a machine that runs on traces
//! built by `trace::build` (reference executor of `testexec.rs`) must end in the state of the
//! interpreter (`Machine::run`, the only exactness oracle) after the same budget: registers,
//! flags, memory, bus pc, instruction count, stop reason.
//!
//! Every scenario also proves it exercised what it is about (the [`Tally`] of the traces that
//! ran), so a pass cannot come from traces that were never entered.

use super::super::config::SbConfig;
use super::super::testgen::{machine_with, tiny_loop, CODE};
use super::super::testkit::snapshot;
use super::super::uop::{k, ExitKind};
use super::shape::{add, b, bcc, bl, mov, BX_LR, MOV_PC_LR};
use super::testexec::{drive, Tally};
use super::BuildOpts;
use crate::machine::Machine;

pub(super) const BX_R3: u32 = 0xE12F_FF13;
pub(super) const MOV_PC_R3: u32 = 0xE1A0_F003;
pub(super) const CMP_R0_R0: u32 = 0xE150_0000;
pub(super) const MOV_R3_BASE: u32 = 0xE3A0_3103; // mov r3,#0xC0000000
pub(super) const ORR_LR_1: u32 = 0xE38E_E001; // orr lr,lr,#1

/// A machine with `setup` applied (random `r0..r7` from `seed`, `r8..r10`, `sp`, `lr` as
/// `machine_with` sets them, interpreter only).
pub(super) fn machine(setup: &dyn Fn(&mut Machine), seed: u64) -> Machine {
    let mut m = machine_with(&[], seed, false);
    setup(&mut m);
    m
}

pub(super) fn put(m: &mut Machine, addr: u32, words: &[u32]) {
    for (i, w) in words.iter().enumerate() {
        m.bus.mem.write32(addr + 4 * i as u32, *w);
    }
}

/// Run `budget` instructions on the interpreter and on traces and demand equal outcomes.
pub(super) fn same(
    label: &str,
    setup: &dyn Fn(&mut Machine),
    seed: u64,
    budget: u64,
    opts: &BuildOpts,
    flips: &[u32],
    tally: &mut Tally,
) {
    let (mut a, mut b) = (machine(setup, seed), machine(setup, seed));
    let ra = a.run(budget, u32::MAX, None);
    let rb = drive(&mut b, budget, opts, flips, tally);
    assert_eq!(ra, rb, "{label}: result, budget {budget}");
    let diff = snapshot(&a, None).diff(&snapshot(&b, None));
    assert_eq!(diff, None, "{label}: state, budget {budget}");
}

/// Option sets that move the caps around the trace shapes; the first is the default.
pub(super) fn option_sets() -> Vec<(&'static str, BuildOpts)> {
    let base = BuildOpts::default();
    let o = |f: &dyn Fn(&mut BuildOpts)| {
        let mut x = base.clone();
        f(&mut x);
        x
    };
    vec![
        ("default", base.clone()),
        ("all passes on", BuildOpts::from(&SbConfig::on())),
        ("no inlining", o(&|x| x.inline_depth = 0)),
        ("depth 1", o(&|x| x.inline_depth = 1)),
        ("no guards", o(&|x| x.max_guards = 0)),
        ("one guard", o(&|x| x.max_guards = 1)),
        ("short", o(&|x| x.max_uops = 4)),
        ("one page", o(&|x| x.max_pages = 1)),
        ("tiny", o(&|x| x.max_uops = 1)),
    ]
}

/// Every budget `1..=span` and two long ones on the default options, a sparse few on the others.
pub(super) fn sweep(span: u64, mut run: impl FnMut(&str, &BuildOpts, u64)) {
    for (i, (name, opts)) in option_sets().iter().enumerate() {
        let budgets: Vec<u64> = if i == 0 {
            (1..=span).chain([100, 333]).collect()
        } else {
            vec![3, 6, 9, 13, 100]
        };
        for budget in budgets {
            run(name, opts, budget);
        }
    }
}

/// `C: bl f ; mov r3,#7 ; b .` and `f: <fix> ; <ret>`: a callee that returns one word late (or
/// to a Thumb address), so an inlined return must leave through its guard.
pub(super) fn late_return(fix: u32, ret: u32) -> impl Fn(&mut Machine) {
    let f = CODE + 0x20;
    move |m| {
        put(m, CODE, &[bl(CODE, f), mov(3, 7), b(CODE + 8, CODE + 8)]);
        put(m, f, &[fix, ret]);
    }
}

/// A store at `C+0xC` rewrites the word at `C+0x10` (`mov r0,#1` becomes `mov r0,#2`), which the
/// trace would run next.
pub(super) fn self_modifying() -> impl Fn(&mut Machine) {
    |m| {
        put(
            m,
            CODE,
            &[
                MOV_R3_BASE,
                add(3, 0x10),
                0xE59F_1004,
                0xE583_1000,
                mov(0, 1),
            ],
        );
        put(m, CODE + 0x14, &[mov(0, 2), b(CODE + 0x18, CODE + 0x18)]);
    }
}

/// `cmp r0,r0 ; bne X` (not taken) `; mov r5,#1 ; beq X` (taken) `; mov r6,#2 ; b .` and
/// `X = C+0x40: mov r7,#3 ; b .`: forward conditional branches on both sides.
pub(super) fn forward_branches() -> impl Fn(&mut Machine) {
    |m| {
        let x = CODE + 0x40;
        let words = [
            CMP_R0_R0,
            bcc(1, CODE + 4, x),
            mov(5, 1),
            bcc(0, CODE + 12, x),
            mov(6, 2),
        ];
        put(m, CODE, &words);
        put(m, CODE + 0x14, &[b(CODE + 0x14, CODE + 0x14)]);
        put(m, x, &[mov(7, 3), b(x + 4, x + 4)]);
    }
}

/// `C: b A` ; `A: mov r3,#0xC0000000 ; add r3,r3,#0x4000 ; mov r0,#1 ; str r0,[r3] ; b B` ;
/// `B: add r0,r0,#1 ; add r1,r1,#2 ; b E` ; `E: b F` ; `F: mov r5,#5 ; b .`, with a junk word
/// behind every branch: unconditional branches that are followed, in front of the trace, behind
/// a store, behind data processing and behind each other.
pub(super) fn followed_branches() -> impl Fn(&mut Machine) {
    |m| {
        let (a, bb, e, f) = (CODE + 0x40, CODE + 0x80, CODE + 0xC0, CODE + 0x100);
        put(m, CODE, &[b(CODE, a), mov(6, 0xEE)]);
        let words = [
            MOV_R3_BASE,
            0xE283_3901,
            mov(0, 1),
            0xE583_0000,
            b(a + 16, bb),
        ];
        put(m, a, &words);
        put(m, a + 20, &[mov(7, 0xEE)]);
        put(m, bb, &[add(0, 1), add(1, 2), b(bb + 8, e), mov(8, 0xEE)]);
        put(m, e, &[b(e, f), mov(9, 0xEE)]);
        put(m, f, &[mov(5, 5), b(f + 4, f + 4)]);
    }
}

/// A short differential run for the mutation tests (`lower_flow/kills.rs`): the call and loop,
/// a late and a Thumb return, a store into the trace and forward branches, on the default
/// options.
pub(in crate::machine::sb) fn probe() {
    let (opts, mut tally, words) = (BuildOpts::default(), Tally::default(), tiny_loop());
    for budget in [9, 10, 11, 12, 40, 100, 333] {
        same(
            "loop",
            &|m| put(m, CODE, &words),
            4,
            budget,
            &opts,
            &[],
            &mut tally,
        );
    }
    for (fix, ret) in [
        (add(14, 4), BX_LR),
        (ORR_LR_1, BX_LR),
        (add(14, 4), MOV_PC_LR),
    ] {
        for budget in [5, 6, 7, 12] {
            same(
                "return",
                &late_return(fix, ret),
                9,
                budget,
                &opts,
                &[],
                &mut tally,
            );
        }
    }
    for budget in [10, 30] {
        same(
            "store",
            &self_modifying(),
            6,
            budget,
            &opts,
            &[],
            &mut tally,
        );
    }
    for budget in [4, 5, 6, 8, 12] {
        same(
            "forward",
            &forward_branches(),
            1,
            budget,
            &opts,
            &[],
            &mut tally,
        );
    }
}

#[test]
fn the_tiny_loop_with_its_call_and_return_equals_the_interpreter_at_every_budget() {
    let words = tiny_loop();
    let mut tally = Tally::default();
    sweep(40, |name, opts, budget| {
        same(
            name,
            &|m| put(m, CODE, &words),
            4,
            budget,
            opts,
            &[],
            &mut tally,
        );
    });
    assert!(tally.traces > 80 && tally.steps > 0, "{tally:?}");
    assert!(
        tally.had(k::GUARD) > 0 && tally.had(k::GUARD_REG_EQ) > 0,
        "{tally:?}"
    );
    assert!(tally.had(k::LOOP_BACK) > 0 && tally.laps > 0, "{tally:?}");
    assert!(
        tally.took(k::GUARD) > 0 && tally.took(k::LOOP_BACK) > 0,
        "{tally:?}"
    );
}

#[test]
fn a_return_that_does_not_come_back_where_the_call_was_inlined_leaves_through_the_guard() {
    let mut tally = Tally::default();
    for (fix, ret) in [
        (add(14, 4), BX_LR),
        (add(14, 4), MOV_PC_LR),
        (ORR_LR_1, BX_LR),
    ] {
        let before = tally.took(k::GUARD_REG_EQ);
        sweep(12, |name, opts, budget| {
            same(
                name,
                &late_return(fix, ret),
                9,
                budget,
                opts,
                &[],
                &mut tally,
            );
        });
        assert!(
            tally.took(k::GUARD_REG_EQ) > before,
            "ret {ret:#x} fix {fix:#x}: {tally:?}"
        );
    }
    assert!(tally.exits[ExitKind::Indirect as usize] > 0, "{tally:?}");
    assert!(tally.exits[ExitKind::IndirectArm as usize] > 0, "{tally:?}");
}

#[test]
fn conditional_and_unconditional_register_jumps_equal_the_interpreter() {
    // C: mov r3,#0xC0000000 ; add r3,r3,#0x40 ; cmp r0,r0 ; <jump> ; mov r5,#1 ; b .
    // C+0x40: mov r4,#9 ; b .
    let mut tally = Tally::default();
    for jump in [
        0x012F_FF13,
        0x112F_FF13,
        0x01A0_F003,
        0x11A0_F003,
        BX_R3,
        MOV_PC_R3,
    ] {
        let setup = |m: &mut Machine| {
            put(
                m,
                CODE,
                &[MOV_R3_BASE, add(3, 0x40), CMP_R0_R0, jump, mov(5, 1)],
            );
            put(m, CODE + 0x14, &[b(CODE + 0x14, CODE + 0x14)]);
            put(m, CODE + 0x40, &[mov(4, 9), b(CODE + 0x44, CODE + 0x44)]);
        };
        sweep(14, |name, opts, budget| {
            same(name, &setup, 5, budget, opts, &[], &mut tally);
        });
    }
    assert!(
        tally.took(k::GUARD) > 0 && tally.took(k::EXIT_REG) > 0,
        "{tally:?}"
    );
    assert!(
        tally.took(k::EXIT_REG_ARM) > 0 && tally.took(k::EXIT) > 0,
        "{tally:?}"
    );
    assert!(tally.had(k::GUARD) > 0, "{tally:?}");
}

#[test]
fn a_counting_loop_runs_whole_laps_and_stays_exact_with_and_without_flipped_prediction() {
    // C: mov r0,#0 ; L: add r0,r0,#1 ; cmp r0,#50 ; bne L ; b .
    let (l, bne) = (CODE + 4, CODE + 12);
    let words = [
        mov(0, 0),
        add(0, 1),
        0xE350_0032,
        bcc(1, bne, l),
        b(CODE + 16, CODE + 16),
    ];
    let setup = |m: &mut Machine| put(m, CODE, &words);
    let (mut plain, mut flipped) = (Tally::default(), Tally::default());
    sweep(14, |name, opts, budget| {
        same(name, &setup, 2, budget, opts, &[], &mut plain);
        same(name, &setup, 2, budget, opts, &[bne], &mut flipped);
    });
    let opts = BuildOpts::default();
    for budget in [160, 200, 201, 202] {
        same("long", &setup, 2, budget, &opts, &[], &mut plain);
        same("long", &setup, 2, budget, &opts, &[bne], &mut flipped);
    }
    assert!(plain.laps > 0 && plain.took(k::LOOP_BACK) > 0, "{plain:?}");
    assert!(
        plain.took(k::GUARD) > 0,
        "the last pass leaves through the guard: {plain:?}"
    );
    assert!(
        flipped.took(k::GUARD) > 20,
        "flipped, the guard is the way back: {flipped:?}"
    );
}

#[test]
fn forward_conditional_branches_taken_and_not_taken_stay_exact_with_every_prediction() {
    let (bne, beq) = (CODE + 4, CODE + 12);
    let mut tally = Tally::default();
    for flips in [vec![], vec![bne], vec![beq], vec![bne, beq]] {
        sweep(12, |name, opts, budget| {
            same(
                name,
                &forward_branches(),
                1,
                budget,
                opts,
                &flips,
                &mut tally,
            );
        });
    }
    assert!(
        tally.took(k::GUARD) > 0 && tally.took(k::EXIT) > 0,
        "{tally:?}"
    );
    assert!(tally.had(k::GUARD) > 0 && tally.laps > 0, "{tally:?}");
}

#[test]
fn followed_unconditional_branches_are_counted_and_stay_exact_at_every_budget() {
    // one trace from `C` covers the 12 instructions and the four followed branches, each behind
    // a no-op, and ends in the exit of `b .`
    let mut one = Tally::default();
    let opts = BuildOpts::default();
    same(
        "one trace",
        &followed_branches(),
        3,
        12,
        &opts,
        &[],
        &mut one,
    );
    let seen = (
        one.traces,
        one.insns,
        one.steps,
        one.had(k::MOV_REG),
        one.took(k::EXIT),
    );
    assert_eq!(seen, (1, 12, 0, 4, 1), "{one:?}");
    let mut tally = Tally::default();
    sweep(16, |name, opts, budget| {
        same(name, &followed_branches(), 3, budget, opts, &[], &mut tally);
    });
    assert!(tally.traces > 80 && tally.steps > 0, "{tally:?}");
    assert!(
        tally.had(k::MOV_REG) > 80 && tally.took(k::EXIT) > 0,
        "{tally:?}"
    );
}
