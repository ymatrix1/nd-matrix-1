//! OWNER: W2-DRV (tests). The driver on synthetic code: the build / hit / invalidate cycle, link
//! patching, a dead trace that is not followed, the build cap, a budget below `n_guest`, `until`
//! and `stop` inside a trace page, a fault inside a trace and a store into the own trace
//! (`plumbing_tests.rs` has the flipped re-layout, the backend results `EXIT_SLOW` and
//! `EXIT_STEPPED`, the tier off and the volatile pages).
//!
//! Every scenario is a plain function that compares the tier against the interpreter
//! (`Machine::run` with `predecode = false`), so `mutant_tests.rs` can replay it under one
//! mutation site and expect a failed assertion.

use super::{run_fast, run_sb_full};
use crate::machine::sb::testgen::asm::{self, AL};
use crate::machine::sb::testkit::compare_machines;
use crate::machine::sb::uop::{ExitKind, NO_SB};
use crate::machine::sb::{SbConfig, SbState};
use crate::machine::{Machine, Stop};

pub(super) const CODE: u32 = 0xC000_0000;
pub(super) const NE: u32 = 1;
pub(super) const EQ: u32 = 0;

/// A machine with `words` at [`CODE`], `pc = CODE`.
pub(super) fn machine(words: &[u32]) -> Machine {
    let mut m = Machine::new();
    for (i, w) in words.iter().enumerate() {
        m.bus.mem.write32(CODE + 4 * i as u32, *w);
    }
    m.cpu.r[15] = CODE;
    m
}

pub(super) fn state(cfg: SbConfig) -> SbState {
    SbState {
        cfg,
        ..SbState::default()
    }
}

/// `want` runs `budget` on the plain interpreter, `got` through `run_sb_full` with `sb`; both
/// results and the whole machine state must be equal.
pub(super) fn compare(want: &mut Machine, got: &mut Machine, sb: &mut SbState, budget: u64) {
    want.predecode = false;
    let w = want.run(budget, 0, None);
    let g = run_sb_full(got, sb, budget, 0, None);
    if let Err(e) = compare_machines(want, w, got, g) {
        panic!("tier differs from the interpreter: {e}");
    }
}

/// `n` times `add r0,r0,#1` then `b CODE`.
fn straight(n: u32) -> Vec<u32> {
    let mut w = vec![asm::add_imm(false, 0, 0, 1); n as usize];
    w.push(asm::branch(AL, false, CODE + 4 * n, CODE));
    w
}

/// `mov r0,#0 ; L: add r0,r0,#1 ; cmp r0,#n ; bne L ; b .`
pub(super) fn counting(n: u32) -> Vec<u32> {
    vec![
        asm::mov_imm(0, 0),
        asm::add_imm(false, 0, 0, 1),
        asm::cmp_imm(0, n),
        asm::branch(NE, false, CODE + 12, CODE + 4),
        asm::branch(AL, false, CODE + 16, CODE + 16),
    ]
}

#[test]
fn a_hot_loop_is_built_entered_and_equals_the_interpreter() {
    let words = counting(200);
    let (mut a, mut b) = (machine(&words), machine(&words));
    let mut sb = state(SbConfig::on());
    compare(&mut a, &mut b, &mut sb, 500);
    assert!(b.cpu.r[0] > 100, "the loop ran: r0 = {}", b.cpu.r[0]);
    assert!(
        sb.stats.built >= 1 && sb.stats.enters >= 1,
        "{:?}",
        sb.stats
    );
    assert!(
        sb.stats.guest_in_traces > 300,
        "the loop ran in traces: {:?}",
        sb.stats
    );
}

/// A page of traces only (tier 1 never decodes it) is patched by the host: the page marks the
/// trace made are the only way the store is noticed.
pub(super) fn a_patch_of_a_traced_page_is_seen_through_the_marks_of_the_trace() {
    let words = [
        asm::add_imm(false, 0, 0, 1),
        asm::add_imm(false, 1, 1, 1),
        asm::branch(AL, false, CODE + 8, CODE),
    ];
    let (mut a, mut b) = (machine(&words), machine(&words));
    let mut sb = state(SbConfig::on());
    compare(&mut a, &mut b, &mut sb, 30);
    assert_eq!((b.cpu.r[0], b.code.live_pages()), (10, 0), "traces only");
    let patched = asm::add_imm(false, 0, 0, 5);
    a.bus.mem.write32(CODE, patched);
    b.bus.mem.write32(CODE, patched);
    compare(&mut a, &mut b, &mut sb, 30);
    assert_eq!(b.cpu.r[0], 10 + 5 * 10);
}

#[test]
fn a_patch_of_a_traced_page_drops_the_trace_through_its_page_mark() {
    a_patch_of_a_traced_page_is_seen_through_the_marks_of_the_trace();
}

pub(super) fn a_patched_instruction_drops_its_trace_and_the_rebuilt_one_runs() {
    let words = counting(250);
    let (mut a, mut b) = (machine(&words), machine(&words));
    let mut sb = state(SbConfig::on());
    compare(&mut a, &mut b, &mut sb, 60);
    assert!(sb.cache.live_count() >= 1, "the loop has a trace");
    // the host patches `add r0,r0,#1` into `add r0,r0,#3` between two calls (a program load)
    let patched = asm::add_imm(false, 0, 0, 3);
    a.bus.mem.write32(CODE + 4, patched);
    b.bus.mem.write32(CODE + 4, patched);
    let dropped = sb.stats.dropped;
    compare(&mut a, &mut b, &mut sb, 300);
    assert!(
        sb.stats.dropped > dropped,
        "the trace read the patched word"
    );
    assert!(
        sb.stats.built >= 2,
        "and a new one was built: {:?}",
        sb.stats
    );
}

#[test]
fn the_build_hit_invalidate_cycle_equals_the_interpreter() {
    a_patched_instruction_drops_its_trace_and_the_rebuilt_one_runs();
}

/// Traces of a few uops each, so a straight line is a chain of traces.
fn chain_cfg() -> SbConfig {
    SbConfig {
        max_uops: 5,
        builds_per_run: 64,
        ..SbConfig::on()
    }
}

/// Run the chain; returns the machine pair and the state.
fn chained() -> (Machine, Machine, SbState) {
    let words = straight(30);
    let (mut a, mut b) = (machine(&words), machine(&words));
    let mut sb = state(chain_cfg());
    compare(&mut a, &mut b, &mut sb, 400);
    (a, b, sb)
}

pub(super) fn a_looked_up_successor_is_linked() {
    let (_, _, sb) = chained();
    let mut linked = 0;
    for (_, t) in sb.cache.iter_live() {
        for e in t.exits.iter().filter(|e| e.kind == ExitKind::Direct) {
            if e.link != NO_SB {
                linked += 1;
                assert_eq!(
                    sb.cache.get(e.link).entry,
                    e.target,
                    "a link starts at its target"
                );
            }
        }
    }
    assert!(sb.cache.live_count() >= 4, "the line is a chain of traces");
    assert!(
        linked >= 3,
        "links were patched after the first lookup: {linked}"
    );
}

#[test]
fn links_are_patched_after_the_first_lookup() {
    a_looked_up_successor_is_linked();
}

pub(super) fn a_dead_trace_is_never_followed_through_a_link() {
    let (mut a, mut b, mut sb) = chained();
    // kill the trace the first one links to, without the link reset the stale path does: its
    // predecessor stays live with a link to a dead trace
    let victim = sb
        .cache
        .iter_live()
        .flat_map(|(_, t)| t.exits.iter().map(|e| e.link).collect::<Vec<_>>())
        .find(|l| *l != NO_SB)
        .expect("the chain has links");
    sb.cache.kill(victim);
    compare(&mut a, &mut b, &mut sb, 300);
}

#[test]
fn a_dead_trace_is_not_followed() {
    a_dead_trace_is_never_followed_through_a_link();
}

#[test]
fn the_build_cap_limits_the_builds_of_one_call() {
    let words = straight(40);
    let mut sb = state(SbConfig {
        builds_per_run: 1,
        ..chain_cfg()
    });
    let mut m = machine(&words);
    assert_eq!(run_fast(&mut m, &mut sb, 400, 0, u32::MAX), None);
    assert_eq!((sb.stats.built, m.cpu.insns), (1, 400));
    assert_eq!(run_fast(&mut m, &mut sb, 800, 0, u32::MAX), None);
    assert_eq!(sb.stats.built, 2, "one more build per call");
    let mut want = machine(&words);
    want.predecode = false;
    want.run(800, 0, None);
    assert_eq!((m.cpu.r, m.cpu.insns), (want.cpu.r, want.cpu.insns));
}

pub(super) fn a_budget_below_n_guest_runs_in_tier_one() {
    let words = straight(8);
    for budget in 1..=8 {
        let (mut a, mut b) = (machine(&words), machine(&words));
        let mut sb = state(SbConfig::on());
        // build first with a budget that covers the trace
        run_sb_full(&mut b, &mut sb, 9, 0, None);
        run_sb_full(&mut a, &mut SbState::default(), 9, 0, None);
        let enters = sb.stats.enters;
        compare(&mut a, &mut b, &mut sb, budget);
        assert_eq!(
            sb.stats.enters, enters,
            "budget {budget} < n_guest 9: no entry"
        );
    }
}

#[test]
fn a_budget_below_n_guest_is_honoured_to_the_instruction() {
    a_budget_below_n_guest_runs_in_tier_one();
}

pub(super) fn until_and_stop_inside_a_trace_page_are_exact() {
    let words = straight(20);
    for (until, stop) in [
        (CODE + 40, None),
        (0, Some(CODE + 28)),
        (CODE + 8, Some(CODE + 4)),
    ] {
        let (mut a, mut b) = (machine(&words), machine(&words));
        let mut sb = state(SbConfig::on());
        a.predecode = false;
        let w = a.run(200, until, stop);
        let g = run_sb_full(&mut b, &mut sb, 200, until, stop);
        compare_machines(&a, w, &b, g).unwrap();
        assert!(matches!(g.1, Stop::Until | Stop::StopPc), "{g:?}");
        assert_eq!(
            sb.stats.enters, 0,
            "the trace page holds the address: tier 1 only"
        );
    }
    // the same code with the address on another page runs in traces
    let (mut b, mut sb) = (machine(&words), state(SbConfig::on()));
    let g = run_sb_full(&mut b, &mut sb, 200, CODE + 0x4000, None);
    assert_eq!(g, (200, Stop::Budget));
    assert!(sb.stats.enters > 0);
}

#[test]
fn until_and_stop_inside_a_trace_page_stop_exactly_there() {
    until_and_stop_inside_a_trace_page_are_exact();
}

pub(super) fn a_fault_inside_a_trace_is_returned_with_the_state_exact() {
    // add ; add ; undefined ; add ; b .   (the fault comes on the 3rd instruction of the trace)
    let words = [
        asm::add_imm(false, 0, 0, 1),
        asm::add_imm(false, 0, 0, 1),
        asm::UNDEFINED,
        asm::add_imm(false, 0, 0, 1),
        asm::branch(AL, false, CODE + 16, CODE + 16),
    ];
    for hot in [0, 2] {
        let (mut a, mut b) = (machine(&words), machine(&words));
        let mut sb = state(SbConfig {
            hot,
            ..SbConfig::on()
        });
        a.predecode = false;
        // twice: the second time the trace exists
        for _ in 0..2 {
            a.cpu.r[15] = CODE;
            b.cpu.r[15] = CODE;
            let w = a.run(50, 0, None);
            let g = run_sb_full(&mut b, &mut sb, 50, 0, None);
            assert!(matches!(w.1, Stop::Fault(_)), "{w:?}");
            compare_machines(&a, w, &b, g).unwrap();
        }
    }
}

#[test]
fn a_fault_inside_a_trace_stops_the_run_like_the_interpreter() {
    a_fault_inside_a_trace_is_returned_with_the_state_exact();
}

#[test]
fn a_store_into_the_own_trace_is_seen_by_the_next_lap() {
    stale_scenario();
}

/// A call into a routine the loop patches: every lap sees the patch of the lap before.
pub(super) fn stale_scenario() {
    // 04 bl T ; 08 str r3,[r2] ; 0c add r4,r4,#1 ; 10 cmp r4,#2 ; 14 bne 04 ; 18 b .  (starts at 04)
    // T at +0x800: add r0,r0,#1 ; bx lr. The str patches T's first word to `add r0,r0,#10`:
    // the trace of 04 holds T inline, so only the drop of that trace shows lap 2 the new word.
    let t = CODE + 0x800;
    let mut words = vec![0u32; 0x202];
    words[1] = asm::branch(AL, true, CODE + 4, t);
    words[2] = asm::str(3, 2, 0);
    words[3] = asm::add_imm(false, 4, 4, 1);
    words[4] = asm::cmp_imm(4, 2);
    words[5] = asm::branch(NE, false, CODE + 20, CODE + 4);
    words[6] = asm::branch(AL, false, CODE + 24, CODE + 24);
    words[0x200] = asm::add_imm(false, 0, 0, 1);
    words[0x201] = asm::bx(AL, 14);
    let (mut a, mut b) = (machine(&words), machine(&words));
    for m in [&mut a, &mut b] {
        (m.cpu.r[2], m.cpu.r[3]) = (t, asm::add_imm(false, 0, 0, 10));
        m.cpu.r[15] = CODE + 4;
    }
    let mut sb = state(SbConfig::on());
    compare(&mut a, &mut b, &mut sb, 40);
    assert_eq!(
        b.cpu.r[0], 11,
        "lap 1 ran the old word, lap 2 the patched one"
    );
    assert!(
        sb.stats.dropped >= 1,
        "the patched word dropped its trace: {:?}",
        sb.stats
    );
}
