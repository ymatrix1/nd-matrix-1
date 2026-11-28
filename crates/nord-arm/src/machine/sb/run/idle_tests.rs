//! OWNER: W2-DRV (tests). Exact idle skipping through the new driver: the schedule of
//! `Machine::run_idle_skip` and `run_tick_idle` with every slice run by `run_sb_full` equals the
//! interpreter's schedule (state hash, instruction count, counters), on a spin loop, a `wfi`, a
//! counting loop and a spin after noise.
//!
//! Before the seam `Machine::run_idle_skip` cannot be pointed at the driver, so the schedule is
//! copied here (the one change is the slice runner). The probes and the timer interrupt run on
//! the interpreter and pop `Mem::stale_code` without telling the state (`Machine::sb` does not
//! exist yet), so the copy flushes the cache after each of them; after the seam the placeholder
//! protocol does that job.

use super::cycle_tests::{machine, state, CODE};
use super::run_sb_full;
use crate::cpu::Fault;
use crate::machine::boot::TIMER_IRQ;
use crate::machine::idle::{IdleStats, MAX_SLICE, MIN_SLICE};
use crate::machine::sb::testgen::asm::{self, AL};
use crate::machine::sb::{SbConfig, SbState};
use crate::machine::{Machine, Stop, LOW_MEMORY, SENTINEL};

/// `Machine::run_idle_skip` with `run_sb_full` as the slice runner.
fn idle_skip(m: &mut Machine, sb: &mut SbState, budget: u64, stats: &mut IdleStats) -> Stop {
    let end = m.cpu.insns + budget;
    let mut slice = MIN_SLICE;
    while m.cpu.insns < end {
        let remaining = end - m.cpu.insns;
        let before = m.cpu.insns;
        let (_, stop) = run_sb_full(m, sb, slice.min(remaining), SENTINEL, m.stop_pc);
        stats.executed += m.cpu.insns - before;
        match stop {
            Stop::Budget => {}
            Stop::Fault(Fault::Wfi) => {
                stats.wfi += 1;
                m.wfi += 1;
                stats.skipped += end - m.cpu.insns;
                m.cpu.insns = end;
                return Stop::Budget;
            }
            other => return other,
        }
        let remaining = end - m.cpu.insns;
        if remaining == 0 {
            break;
        }
        stats.probes += 1;
        let before = m.cpu.insns;
        let (period, stop) = m.probe_idle(remaining);
        sb.flush_all();
        stats.executed += m.cpu.insns - before;
        stats.probe_executed += m.cpu.insns - before;
        if let Some(Stop::Fault(Fault::Wfi)) = stop {
            stats.wfi += 1;
            m.wfi += 1;
            stats.skipped += end - m.cpu.insns;
            m.cpu.insns = end;
            return Stop::Budget;
        }
        if let Some(s) = stop {
            return s;
        }
        match period {
            Some(l) => {
                let k = (end - m.cpu.insns) / l;
                m.cpu.insns += k * l;
                stats.skips += 1;
                stats.skipped += k * l;
                slice = MIN_SLICE;
            }
            None => slice = (slice * 2).min(MAX_SLICE),
        }
    }
    Stop::Budget
}

/// `Machine::run_tick_idle` over [`idle_skip`].
fn tick(m: &mut Machine, sb: &mut SbState, insns: u64, stats: &mut IdleStats) -> Stop {
    let stop = idle_skip(m, sb, insns, stats);
    if matches!(stop, Stop::Budget) {
        m.bus.mem.zero(0, LOW_MEMORY as usize);
        m.inject_irq(TIMER_IRQ);
        sb.flush_all();
    }
    stop
}

fn programs() -> Vec<(&'static str, Vec<u32>)> {
    vec![
        // cmp r1,#0 ; moveq r2,r2 ; b .-8 : a pure spin of three instructions
        (
            "spin",
            vec![
                0xE351_0000,
                0x01A0_2002,
                asm::branch(AL, false, CODE + 8, CODE),
            ],
        ),
        // wfi ; add r1,r1,#1 ; b .
        ("wfi", vec![asm::WFI, 0xE281_1001, 0xEAFF_FFFE]),
        // add r0,r0,#1 ; b .-4 : never idle, the registers change
        (
            "count",
            vec![0xE280_0001, asm::branch(AL, false, CODE + 4, CODE)],
        ),
        // 20 adds of noise, then the spin
        ("noise", {
            let mut w = vec![asm::add_imm(false, 3, 3, 1); 20];
            let at = CODE + 4 * w.len() as u32;
            w.extend([0xE351_0000, 0x01A0_2002, asm::branch(AL, false, at + 8, at)]);
            w
        }),
    ]
}

#[test]
fn ticks_through_the_driver_equal_the_interpreter_schedule() {
    let (mut skips, mut entered, mut ticks) = (0, 0, 0);
    for (name, code) in programs() {
        for cfg in [
            SbConfig::on(),
            SbConfig {
                hot: 3,
                ..SbConfig::on()
            },
        ] {
            let (mut a, mut b) = (machine(&code), machine(&code));
            let (mut sa, mut sbs) = (IdleStats::default(), IdleStats::default());
            let mut sb = state(cfg);
            for t in 0..6 {
                let ra = a.run_tick_idle(20_000, &mut sa);
                let rb = tick(&mut b, &mut sb, 20_000, &mut sbs);
                assert_eq!((ra, sa), (rb, sbs), "{name} tick {t}");
                assert_eq!(
                    (a.wfi, a.cpu.insns, a.cpu.r, a.state_hash()),
                    (b.wfi, b.cpu.insns, b.cpu.r, b.state_hash()),
                    "{name} tick {t}"
                );
                ticks += 1;
            }
            skips += sa.skips + sa.wfi;
            entered += sb.stats.enters;
        }
    }
    assert!(
        ticks == 48 && skips >= 20,
        "idle skipping happened: {skips}"
    );
    assert!(
        entered > 0,
        "and the driver ran traces between the probes: {entered}"
    );
}
