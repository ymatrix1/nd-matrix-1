//! Tick-level differential tests of idle v2 stage 1: whole `run_idle_skip_v2` ticks against
//! `Machine::run_idle_skip`, on the six idle tests of `machine/idle.rs` and on generated programs.
//! The helpers are in `diff.rs`. The sweep includes programs whose counting stretch outlasts the
//! point where the v2 slices part from v1's, and one test pins that case on its own.

use super::diff::tick_both;
use super::progs::{self, machine, mov_imm, Tail, AL, SCHEDULES_PART_AFTER, SPIN1, WFI};
use crate::machine::sb::testkit::{fuzz_cases, fuzz_seed, Rng};
use crate::machine::Stop;

#[test]
fn the_six_idle_tests_of_v1_end_in_the_same_state_through_v2() {
    // 1. a pure loop is skipped
    let p = tick_both(
        &machine(&[0xE351_0000, 0x01A0_2002, 0xEAFF_FFFC]),
        100_001,
        "pure loop",
    );
    assert!(
        p.s2.skips >= 1 && p.s2.skipped > 99_000 && p.s2.executed < 1_000,
        "{:?}",
        p.s2
    );
    // 2. a counting loop is not
    let p = tick_both(
        &machine(&[0xE280_0001, 0xEAFF_FFFD]),
        10_000,
        "counting loop",
    );
    assert_eq!((p.s2.skipped, p.v2.reg(0)), (0, 5_000));
    // 3. an idempotent store is skipped, a counter in memory is not
    let p = tick_both(
        &machine(&[0xE3A0_1A02, 0xE581_0000, 0xEAFF_FFFD]),
        10_000,
        "idempotent store",
    );
    assert!(p.s2.skipped > 9_000, "{:?}", p.s2);
    let code = [
        0xE3A0_1A02,
        0xE591_0000,
        0xE280_0001,
        0xE581_0000,
        0xEAFF_FFFB,
    ];
    let mut p = tick_both(&machine(&code), 10_000, "memory counter");
    assert_eq!(p.s2.skipped, 0);
    assert!(p.v2.bus.mem.read32(0x2000) > 2_000);
    // 4. a device poll is never skipped
    let p = tick_both(
        &machine(&[0xE3A0_1206, 0xE591_0000, 0xEAFF_FFFD]),
        5_000,
        "device poll",
    );
    assert_eq!(p.s2.skipped, 0);
    assert!(p.v2.bus.device_reads > 1_000 && p.v2.bus.device_reads == p.v1.bus.device_reads);
    // 5. WFI skips the rest of the tick
    let p = tick_both(&machine(&[WFI, SPIN1]), 200_000, "wfi");
    assert_eq!(
        (p.s2.wfi, p.s2.skipped, p.v2.cpu.insns),
        (1, 199_999, 200_000)
    );
    // 6. WFI is one semantic in the full path and the skipping path
    let m = machine(&[WFI, 0xE281_1001, SPIN1]);
    let p = tick_both(&m, 10_000, "wfi then add");
    let mut full = m.clone();
    assert_eq!(
        full.run_wfi(10_000, crate::machine::SENTINEL, None),
        (10_000, Stop::Budget)
    );
    assert_eq!((p.v2.wfi, full.wfi), (1, 1));
    assert_eq!(p.v2.state_hash(), full.state_hash());
    assert_eq!((p.v2.cpu.insns, p.v2.reg(1)), (full.cpu.insns, full.reg(1)));
}

#[test]
fn random_programs_end_in_the_same_state_under_both_schedules() {
    let mut rng = Rng::new(0x1D1E_5EED ^ fuzz_seed());
    let (mut skipped, mut cmp_v2, mut cmp_v1_min, mut forced, mut periods) =
        (0u64, 0u64, 0u64, 0, 0u64);
    // long-stretch programs on which the two schedules differ (and both skipped)
    let mut parted = 0;
    let cases = fuzz_cases(200).max(200);
    for i in 0..cases {
        let prog = progs::random_prog(&mut rng, i);
        let what = format!(
            "program {i} {:?} budget {} work {}",
            prog.tail, prog.budget, prog.work
        );
        let p = tick_both(&machine(&prog.code), prog.budget, &what);
        assert_eq!(p.stop, Stop::Budget, "{what}");
        match prog.tail {
            Tail::Device => assert_eq!(
                (p.s1.skipped, p.s2.skipped),
                (0, 0),
                "{what}: a device poll is never idle"
            ),
            Tail::Wfi if prog.budget > prog.work + 2 => {
                assert_eq!((p.v1.wfi, p.v2.wfi), (1, 1), "{what}")
            }
            Tail::Spin(n) if prog.must_skip => periods |= 1 << (n - 1),
            _ => {}
        }
        if prog.must_skip {
            assert!(
                p.s1.skips >= 1 && p.s2.skips >= 1,
                "{what}: {:?} {:?}",
                p.s1,
                p.s2
            );
            forced += 1;
        }
        let schedule = |s: &crate::machine::idle::IdleStats| (s.probes, s.executed);
        parted += (prog.work > SCHEDULES_PART_AFTER
            && prog.must_skip
            && schedule(&p.s1) != schedule(&p.s2)) as u32;
        skipped += p.s2.skipped;
        cmp_v2 += p.c2.compares;
        cmp_v1_min += p.s1.probe_executed.saturating_sub(p.s1.probes);
    }
    assert!(forced >= 40, "programs that had to skip: {forced}");
    // a long stretch makes v2 probe less often and find an idle tail later, and both still skip it
    assert!(
        parted >= 5,
        "long programs with a different v2 schedule that still skipped: {parted}"
    );
    assert!(skipped > 1_000_000, "skipped instructions: {skipped}");
    assert!(
        periods.count_ones() >= 20,
        "spin periods exercised: {periods:#x}"
    );
    // v1 compares after every probed instruction but one per probe at most; v2 once per visit home
    assert!(
        cmp_v2 * 2 < cmp_v1_min,
        "v2 compares {cmp_v2} against at least {cmp_v1_min} for v1"
    );
}

/// `mov r10,#0x2000 ; mov r4,#(1 << n) ; top: subs r4,r4,#1 ; bne top ; <tail>` for an even `n`,
/// and the instructions of the counting stretch (the `mov`s plus `2 << n`). Every tail word is
/// position independent (relative branches only).
fn stretch_then(n: u32, tail: &[u32]) -> (crate::machine::Machine, u64) {
    let mut code = vec![
        0xE3A0_AA02,
        mov_imm(AL, 4, 1, (32 - n) / 2),
        0xE254_4001,
        progs::branch(1, 3, 2),
    ];
    code.extend_from_slice(tail);
    (machine(&code), 2 + (2 << n))
}

#[test]
fn a_long_counting_stretch_moves_the_v2_schedule_and_both_still_skip() {
    // the four idle tails: a period-3 spin, `b .`, a memory poll that never changes, a WFI
    let tails: [(&str, &[u32]); 4] = [
        ("spin3", &[0xE351_0000, 0x01A0_2002, 0xEAFF_FFFC]),
        ("b .", &[SPIN1]),
        ("poll", &[0xE59A_5000, 0xE355_0000, 0x0AFF_FFFC, SPIN1]),
        ("wfi", &[WFI, SPIN1]),
    ];
    let (mut parted, mut same, mut later) = (0, 0, 0);
    for n in [12, 14, 16, 18] {
        for (name, tail) in tails {
            for budget in [400_000u64, 1_000_000, 3_000_000] {
                let (m, work) = stretch_then(n, tail);
                let what = format!("2^{n} iterations ({work} insns) then {name}, budget {budget}");
                let p = tick_both(&m, budget, &what);
                assert_eq!(p.stop, Stop::Budget, "{what}");
                // the budget leaves room for either schedule to reach the tail and skip
                if budget >= 3 * work + 30_000 {
                    if name == "wfi" {
                        assert_eq!((p.s1.wfi, p.s2.wfi), (1, 1), "{what}");
                    } else {
                        assert!(
                            p.s1.skips >= 1 && p.s2.skips >= 1,
                            "{what}: {:?} {:?}",
                            p.s1,
                            p.s2
                        );
                        assert!(p.s2.skipped > budget - work - 100_000, "{what}: {:?}", p.s2);
                    }
                    if work > SCHEDULES_PART_AFTER {
                        // v2 probes less often through the stretch, so it finds the tail later
                        assert!(p.s2.probes < p.s1.probes, "{what}: {:?} {:?}", p.s1, p.s2);
                        parted += 1;
                        later += (name != "wfi" && p.s2.executed > p.s1.executed) as u32;
                    } else {
                        // the stretch ends before the schedules part: the same probes, run for run
                        assert_eq!(
                            (p.s1.probes, p.s1.executed),
                            (p.s2.probes, p.s2.executed),
                            "{what}"
                        );
                        same += 1;
                    }
                }
            }
        }
    }
    assert!(
        parted >= 12 && same >= 12 && later >= 9,
        "{parted} {same} {later}"
    );
}

#[test]
fn a_tick_that_ends_before_v2_reaches_the_tail_equals_one_that_skipped_it_in_v1() {
    // 2^16 iterations (131 074 insns) then a period-3 spin. v1 probes at 143 299 and skips; v2's
    // 65 536-instruction slice runs from 106 k to the end of the 150 k budget, so it never
    // probes: it executes the tail for real and ends in the very same state.
    let (m, _) = stretch_then(16, &[0xE351_0000, 0x01A0_2002, 0xEAFF_FFFC]);
    let p = tick_both(&m, 150_000, "v1 skips, v2 does not");
    assert_eq!((p.s1.skips, p.s2.skips), (1, 0), "{:?} {:?}", p.s1, p.s2);
    assert_eq!((p.s1.executed < 150_000, p.s2.executed), (true, 150_000));
    // the same tick one slice later: both skip
    let p = tick_both(&m, 200_000, "both skip");
    assert_eq!((p.s1.skips, p.s2.skips), (1, 1), "{:?} {:?}", p.s1, p.s2);
}
