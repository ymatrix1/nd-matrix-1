//! Tick-level differential tests of idle v2 stage 1: whole `run_idle_skip_v2` ticks against
//! `Machine::run_idle_skip`, on the six idle tests of `machine/idle.rs` and on generated programs.
//! The helpers are in `diff.rs`.

use super::diff::tick_both;
use super::progs::{self, machine, Tail, SPIN1, WFI};
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
        skipped += p.s2.skipped;
        cmp_v2 += p.c2.compares;
        cmp_v1_min += p.s1.probe_executed.saturating_sub(p.s1.probes);
    }
    assert!(forced >= 40, "programs that had to skip: {forced}");
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
