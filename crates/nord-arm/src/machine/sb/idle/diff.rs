//! Differential tests of the idle v2 probe (and the comparison helpers of `ticks.rs`) against the
//! v1 path (`Machine::probe_idle`, `Machine::run_idle_skip`), the oracle. Each test also asserts that it exercised something
//! (a period found, a skip taken, a stop reached), so none can pass by never looking.

use super::progs::{self, machine, CODE, SPIN1, WFI};
use super::{probe_idle_v2_counted, run_idle_skip_v2_counted, V2Counters};
use crate::cpu::Fault;
use crate::machine::idle::IdleStats;
use crate::machine::sb::testkit::{fuzz_seed, snapshot, Rng};
use crate::machine::{Hook, Machine, Stop};

/// Both schedules over copies of one starting state.
pub(super) struct Pair {
    pub v1: Machine,
    pub v2: Machine,
    pub stop: Stop,
    pub s1: IdleStats,
    pub s2: IdleStats,
    pub c2: V2Counters,
}

/// Run `budget` instructions of virtual time through `run_idle_skip` (v1) and
/// `run_idle_skip_v2_counted` (v2) and require the same stop and the same final state.
pub(super) fn tick_both(m: &Machine, budget: u64, what: &str) -> Pair {
    let (mut v1, mut v2) = (m.clone(), m.clone());
    let (mut s1, mut s2, mut c2) = (
        IdleStats::default(),
        IdleStats::default(),
        V2Counters::default(),
    );
    let stop1 = v1.run_idle_skip(budget, &mut s1);
    let stop2 = run_idle_skip_v2_counted(&mut v2, budget, &mut s2, &mut c2);
    assert_eq!(stop1, stop2, "{what}: stop");
    let fault = if let Stop::Fault(f) = stop1 {
        Some(f)
    } else {
        None
    };
    let d = snapshot(&v1, fault).diff(&snapshot(&v2, fault));
    assert_eq!(d, None, "{what}: final state");
    assert!(
        !v2.bus.probing && !v1.bus.probing,
        "{what}: probe flag left on"
    );
    if stop1 == Stop::Budget {
        // every instruction of the tick is either executed or skipped, in both schedules
        assert_eq!(
            s1.executed + s1.skipped,
            budget,
            "{what}: v1 accounting {s1:?}"
        );
        assert_eq!(
            s2.executed + s2.skipped,
            budget,
            "{what}: v2 accounting {s2:?}"
        );
    }
    Pair {
        v1,
        v2,
        stop: stop1,
        s1,
        s2,
        c2,
    }
}

/// One probe of v1 on `v1` and of v2 on `v2` (two machines in the same state): the same result,
/// the same state afterwards. Returns the result, the v2 counters and the instructions probed.
fn probe_pair(
    v1: &mut Machine,
    v2: &mut Machine,
    limit: u64,
    what: &str,
) -> ((Option<u64>, Option<Stop>), V2Counters, u64) {
    let before = v1.cpu.insns;
    let mut c = V2Counters::default();
    let r1 = v1.probe_idle(limit);
    let r2 = probe_idle_v2_counted(v2, limit, &mut c);
    assert_eq!(r1, r2, "{what}: result");
    let fault = if let (_, Some(Stop::Fault(f))) = r1 {
        Some(f)
    } else {
        None
    };
    assert_eq!(
        snapshot(v1, fault).diff(&snapshot(v2, fault)),
        None,
        "{what}: state"
    );
    assert_eq!(
        (v1.bus.impure, v1.bus.probing),
        (v2.bus.impure, v2.bus.probing),
        "{what}: bus flags"
    );
    (r1, c, v1.cpu.insns - before)
}

/// [`probe_pair`] on two copies of `m`.
fn probe_both(
    m: &Machine,
    limit: u64,
    what: &str,
) -> ((Option<u64>, Option<Stop>), V2Counters, u64) {
    probe_pair(&mut m.clone(), &mut m.clone(), limit, what)
}

#[test]
fn the_probe_equals_v1_for_every_spin_period_at_every_phase() {
    let mut rng = Rng::new(0x1D1E ^ fuzz_seed());
    let (mut found, mut cmp_v2, mut cmp_v1) = (0, 0, 0);
    for period in 1..=40u64 {
        let loop_code = progs::spin_loop(&mut rng, period as u32);
        // two machines in lockstep, one instruction further each round (a found loop returns to
        // its state, so the rounds visit every phase): first while the loop settles, then long after
        for (warm_up, rounds) in [(0, 2 * period + 4), (20 * period + 50, period)] {
            let (mut v1, mut v2) = (machine(&loop_code), machine(&loop_code));
            for m in [&mut v1, &mut v2] {
                m.run(warm_up, 0, None);
            }
            for round in 0..rounds {
                let what = format!("period {period}, warm-up {warm_up}, round {round}");
                let ((result, stop), c, ran) = probe_pair(&mut v1, &mut v2, 4096, &what);
                assert_eq!(stop, None, "{what}");
                if let Some(l) = result {
                    assert_eq!(
                        l, ran,
                        "{what}: the loop closes on the last probed instruction"
                    );
                    assert!((1..=l).contains(&c.compares), "{what}: {c:?}");
                    (found, cmp_v1, cmp_v2) = (found + 1, cmp_v1 + l, cmp_v2 + c.compares);
                }
                if warm_up > 0 {
                    // v1 compares after each of the `l` instructions; v2 once per visit of the start pc
                    let l =
                        result.unwrap_or_else(|| panic!("{what}: a settled loop must be found"));
                    assert_eq!((l % period, c.compares), (0, l / period), "{what}");
                }
                for m in [&mut v1, &mut v2] {
                    m.run(1, 0, None);
                }
            }
        }
    }
    assert!(found > 150, "the sweep must find loops: {found}");
    assert!(
        cmp_v2 * 3 < cmp_v1,
        "v2 compares {cmp_v2} against v1 {cmp_v1}"
    );
}

#[test]
fn a_stop_pc_ends_the_probe_where_v1_ends_it_and_only_there() {
    let code = [0xE280_0001, 0xE280_0001, 0xE280_0001, 0xEAFF_FFFC]; // add x3 ; b top
    let stop_at = |off: u32| {
        let mut m = machine(&code);
        m.stop_pc = Some(CODE + off);
        probe_both(&m, 4096, "stop")
    };
    // inside the loop: after executing the words before it; at the start pc: before any
    assert_eq!(
        stop_at(8),
        (
            (None, Some(Stop::StopPc)),
            V2Counters {
                compares: 0,
                page_checks: 1
            },
            2
        )
    );
    assert_eq!(stop_at(0).0, (None, Some(Stop::StopPc)));
    assert_eq!(stop_at(0).2, 0);
    // on the same page but never executed, and on another page: the probe runs its 4096
    for off in [0x800, 0x1000, 0x2000_0000] {
        let ((period, stop), _, ran) = stop_at(off);
        assert_eq!((period, stop, ran), (None, None, 4096), "{off:#x}");
    }
}

#[test]
fn a_hooked_page_ends_the_probe_before_its_first_instruction() {
    // a hook elsewhere on the code page hooks the whole page (HOOK_X is page-granular)
    let mut m = machine(&[0xE280_0001, 0xEAFF_FFFD]);
    m.add_hook(CODE + 0x400, Hook::SkipInsn);
    assert_eq!(
        probe_both(&m, 4096, "hooked start"),
        (
            (None, None),
            V2Counters {
                compares: 0,
                page_checks: 1
            },
            0
        )
    );
    // a branch into a hooked page: the add and the branch run, the target does not
    let mut m = machine(&[0xE280_0001, 0xEA00_03FD]); // b CODE + 0x1000
    m.add_hook(CODE + 0x1000, Hook::SkipInsn);
    let (r, c, ran) = probe_both(&m, 4096, "hooked target");
    assert_eq!((r, c.page_checks, ran), ((None, None), 2, 2));
    // the same tick through run_idle_skip ends in the same state (the hook runs in `run`, the
    // zero words after it are executed for real)
    assert_eq!(tick_both(&m, 3_000, "hooked tick").s2.skipped, 0);
}

#[test]
fn faults_wfi_thumb_and_the_plain_step_path_match_v1() {
    let swi = [0xE280_0001, 0xEF00_0007, SPIN1];
    let undefined = [0xE280_0001, 0xE7F0_00F0, SPIN1];
    for (code, what) in [(&swi[..], "swi"), (&undefined[..], "undefined")] {
        let m = machine(code);
        let (r, _, ran) = probe_both(&m, 4096, what);
        assert!(
            matches!(
                r,
                (
                    None,
                    Some(Stop::Fault(Fault::Swi { .. } | Fault::Undefined { .. }))
                )
            ),
            "{what}: {r:?}"
        );
        assert_eq!(ran, 2, "{what}");
        assert!(
            matches!(tick_both(&m, 50, what).stop, Stop::Fault(_)),
            "{what}"
        );
    }
    let (r, _, ran) = probe_both(&machine(&[0xE280_0001, WFI, SPIN1]), 4096, "wfi");
    assert_eq!((r, ran), ((None, Some(Stop::Fault(Fault::Wfi))), 2));
    // Thumb state (`b .` is 0xE7FE) and `predecode = false` take the plain `Cpu::step` path
    let mut m = machine(&[0xE7FE]);
    m.cpu.thumb = true;
    assert_eq!(probe_both(&m, 100, "thumb").0, (Some(1), None));
    let p = tick_both(&m, 10_000, "thumb tick");
    assert!(p.s2.skipped > 9_000, "{:?}", p.s2);
    let mut m = machine(&[0xE351_0000, 0x01A0_2002, 0xEAFF_FFFC]);
    m.predecode = false;
    m.run(10, 0, None);
    assert_eq!(probe_both(&m, 100, "no predecode").0, (Some(3), None));
}

#[test]
fn impure_instructions_end_the_probe_at_the_same_instruction() {
    // mov r1,#0x2000 ; mov r3,#0x6000_0000 ; mov r4,#1 ; <op> ; b <op>   (r2 is set below)
    let setup = [0xE3A0_1A02, 0xE3A0_3206, 0xE3A0_4001];
    let cases: [(&str, u32, bool); 4] = [
        ("changing store", 0xE581_4000, false),  // str r4,[r1]
        ("device read", 0xE593_0000, false),     // ldr r0,[r3]
        ("unmapped read", 0xE592_0000, false),   // ldr r0,[r2]
        ("idempotent store", 0xE581_0000, true), // str r0,[r1]
    ];
    for (what, word, pure) in cases {
        let mut m = machine(&[setup[0], setup[1], setup[2], word, 0xEAFF_FFFD]);
        m.run(3, 0, None);
        m.bus.mem.map(0x2000, 0x1000); // a first touch of an unmapped page is impure by itself
        m.cpu.r[2] = 0xC123_0000;
        let (r, _, ran) = probe_both(&m, 4096, what);
        assert_eq!(
            (r, ran),
            if pure {
                ((Some(2), None), 2)
            } else {
                ((None, None), 1)
            },
            "{what}"
        );
    }
}

#[test]
fn a_loop_across_a_page_boundary_rechecks_the_page_on_each_change() {
    // two words at the end of one page and two at the start of the next, then `b top`
    let on_boundary = |first: u32| {
        let mut m = Machine::new();
        let top = CODE + 0xFF8;
        let words = [first, 0xE351_0000, 0xE352_0000, 0xEA00_0000 | 0x00FF_FFFB];
        for (i, w) in words.iter().enumerate() {
            m.bus.mem.write32(top + 4 * i as u32, *w);
        }
        m.cpu.r[15] = top;
        m.run(8, 0, None);
        m
    };
    // cmp r0,#0 first: pure. The loop closes on its first return home: pages A, B
    let (r, c, _) = probe_both(&on_boundary(0xE350_0000), 4096, "pure page loop");
    assert_eq!(
        (r, c),
        (
            (Some(4), None),
            V2Counters {
                compares: 1,
                page_checks: 2
            }
        )
    );
    // add r0,r0,#1 first: never pure. 12 probed instructions: home at 4, 8 and 12, pages A B A B A B
    let (r, c, _) = probe_both(&on_boundary(0xE280_0001), 12, "counting page loop");
    assert_eq!(
        (r, c),
        (
            (None, None),
            V2Counters {
                compares: 3,
                page_checks: 6
            }
        )
    );
}
