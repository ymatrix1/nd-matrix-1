//! OWNER: W1-IDLE. Idle skipping v2, stage 1: tier 1 only, exact by construction.
//!
//! Spec (design note 5, WP W1-IDLE). Three changes to the schedule of `Machine::run_idle_skip`,
//! none to the skip rule (`k * L` instructions, exactly as in `machine/idle.rs`):
//!
//! | Change | Why it cannot change a result |
//! |---|---|
//! | [`probe_idle_v2`] calls `same_state` only when `r[15]` equals the probe's start pc | `same_state` compares `r`, which contains `r[15]`, so an equal pc is a necessary condition: the first matching instant is the one v1 finds, with the same period |
//! | the `HOOK_X` and `stop_pc` checks run when the pc enters a new page, not per instruction | the flag is page-granular and only the host sets it (`add_hook`, `guard_null_calls`), never guest execution, so it cannot change inside a probe; `stop_pc` is compared on its own page only, where equality is possible |
//! | after a failed probe the next slice doubles up to [`MAX_SLICE_V2`] (v1 stops at 8192) | the slice only delays the next probe; the instructions it runs are real execution, so the state after any number of them is the interpreter's |
//!
//! Reused: `machine::idle::same_state` (the equality), `Machine::step_cached` (the probed
//! instruction) and `Machine::run` (the slices).
//!
//! Deviation from the work-package row ("reuse `Machine::probe_loop`, never a copy"):
//! [`probe_loop_v2`] mirrors `Machine::probe_loop` (and the two functions around it,
//! `probe_idle` and `run_idle_skip`, are mirrored by [`probe_idle_v2_counted`] and
//! [`run_idle_skip_v2_counted`]). Stage 1 changes the body of that loop (the state compare, the
//! hook and the stop checks), so it cannot be called as is, and `machine/idle.rs` is not owned by
//! this package. The drift risk is covered three ways: every probe result and every final
//! machine state is compared against `Machine::probe_idle` and `Machine::run_idle_skip` in
//! `idle/diff.rs` and `idle/ticks.rs`; `idle/drift.rs` pins the text of the three v1 functions,
//! so editing one of them fails a test that names the mirror to update; and the follow-up
//! (needs, W1-IDLE) is to extract the per-instruction step of `probe_loop` into a `pub(super)`
//! method that both loops call, which removes the copy of the part that matters.
//!
//! [`V2Counters`] holds the deterministic counters of stage 1 next to `IdleStats` (whose layout
//! the other work packages and the bench share).

use crate::cpu::{Cpu, Fault};
use crate::machine::idle::{same_state, IdleStats, MAX_PERIOD, MIN_SLICE};
use crate::machine::{Machine, Stop, SENTINEL};
use crate::mem::HOOK_X;

#[cfg(test)]
mod diff;
#[cfg(test)]
mod drift;
#[cfg(test)]
mod progs;
#[cfg(test)]
mod ticks;

/// Ceiling of the slice between two probes: 2^20 instructions.
///
/// ASSUMED: chosen from the design note (section 5), not measured. v1 stops doubling at 8192, so
/// a long non-idle stretch pays one probe of up to `MAX_PERIOD` (4096) instructions per 8192;
/// 2^20 makes that 128 times rarer. A larger slice only delays the detection of a loop that
/// starts inside it (by at most the slice, never past the tick end). The gate G0 counters
/// (`probe_executed / executed` per phase) confirm or refute it.
pub const MAX_SLICE_V2: u64 = 1 << 20;

/// Deterministic counters of idle v2 stage 1 (no wall clock).
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct V2Counters {
    /// `same_state` calls. v1 makes one per probed instruction that was not the last; v2 one per
    /// probed instruction whose pc is the probe's start pc.
    pub compares: u64,
    /// Pages entered by the probe, i.e. evaluations of the `HOOK_X` and `stop_pc` checks (v1
    /// evaluates both per instruction).
    pub page_checks: u64,
}

/// Execute up to `MAX_PERIOD` instructions looking for a pure loop (contract of
/// `Machine::probe_idle`): the loop period when found, else `None` plus an optional stop.
pub fn probe_idle_v2(m: &mut Machine, limit: u64) -> (Option<u64>, Option<Stop>) {
    probe_idle_v2_counted(m, limit, &mut V2Counters::default())
}

/// [`probe_idle_v2`] that adds its work to `c`.
pub fn probe_idle_v2_counted(
    m: &mut Machine,
    limit: u64,
    c: &mut V2Counters,
) -> (Option<u64>, Option<Stop>) {
    let start = m.cpu.clone();
    m.bus.probing = true;
    m.bus.impure = false;
    let out = probe_loop_v2(m, &start, limit.min(MAX_PERIOD), c);
    m.bus.probing = false;
    out
}

/// The loop of `Machine::probe_loop` with the per-instruction work hoisted (see the module doc).
fn probe_loop_v2(
    m: &mut Machine,
    start: &Cpu,
    n: u64,
    c: &mut V2Counters,
) -> (Option<u64>, Option<Stop>) {
    let home = start.r[15];
    let stop_pc = m.stop_pc;
    let (mut page, mut hooked, mut stop_here) = (u32::MAX, false, false);
    for i in 1..=n {
        let pc = m.cpu.r[15];
        if pc >> 12 != page {
            page = pc >> 12;
            c.page_checks += 1;
            hooked = m.bus.mem.flags(pc) & HOOK_X != 0;
            stop_here = stop_pc.is_some_and(|s| s >> 12 == page);
        }
        if stop_here && Some(pc) == stop_pc {
            return (None, Some(Stop::StopPc));
        }
        if hooked {
            return (None, None);
        }
        m.bus.pc = pc;
        let fault = if m.predecode && !m.cpu.thumb && pc & 3 == 0 {
            m.step_cached(pc)
        } else {
            m.cpu.step(&mut m.bus).err()
        };
        if let Some(f) = fault {
            return (None, Some(Stop::Fault(f)));
        }
        if m.bus.impure {
            return (None, None);
        }
        if m.cpu.r[15] == home {
            c.compares += 1;
            if same_state(&m.cpu, start) {
                return (Some(i), None);
            }
        }
    }
    (None, None)
}

/// A `WFI` ends the tick: virtual time jumps to `end` (the one semantic of every run path).
fn wake_at_end(m: &mut Machine, end: u64, stats: &mut IdleStats) -> Stop {
    stats.wfi += 1;
    m.wfi += 1;
    stats.skipped += end - m.cpu.insns;
    m.cpu.insns = end;
    Stop::Budget
}

/// Run `budget` instructions of virtual time, skipping provably idle loops (contract of
/// `Machine::run_idle_skip`).
pub fn run_idle_skip_v2(m: &mut Machine, budget: u64, stats: &mut IdleStats) -> Stop {
    run_idle_skip_v2_counted(m, budget, stats, &mut V2Counters::default())
}

/// [`run_idle_skip_v2`] that adds the probe work to `c`.
pub fn run_idle_skip_v2_counted(
    m: &mut Machine,
    budget: u64,
    stats: &mut IdleStats,
    c: &mut V2Counters,
) -> Stop {
    let end = m.cpu.insns + budget;
    let mut slice = MIN_SLICE;
    while m.cpu.insns < end {
        let before = m.cpu.insns;
        let (_, stop) = m.run(slice.min(end - before), SENTINEL, m.stop_pc);
        stats.executed += m.cpu.insns - before;
        match stop {
            Stop::Budget => {}
            Stop::Fault(Fault::Wfi) => return wake_at_end(m, end, stats),
            other => return other,
        }
        if m.cpu.insns >= end {
            break;
        }
        stats.probes += 1;
        let before = m.cpu.insns;
        let (period, stop) = probe_idle_v2_counted(m, end - before, c);
        let ran = m.cpu.insns - before;
        stats.executed += ran;
        stats.probe_executed += ran;
        match stop {
            Some(Stop::Fault(Fault::Wfi)) => return wake_at_end(m, end, stats),
            Some(s) => return s,
            None => {}
        }
        match period {
            Some(l) => {
                let k = (end - m.cpu.insns) / l;
                m.cpu.insns += k * l;
                stats.skips += 1;
                stats.skipped += k * l;
                slice = MIN_SLICE;
            }
            None => slice = (slice * 2).min(MAX_SLICE_V2),
        }
    }
    Stop::Budget
}

#[cfg(test)]
mod tests {
    use super::progs::machine;
    use super::*;

    /// loop: cmp r1,#0 ; moveq r2,r2 ; b loop
    const SPIN3: [u32; 3] = [0xE351_0000, 0x01A0_2002, 0xEAFF_FFFC];

    #[test]
    fn v2_skips_a_pure_loop_exactly_like_the_v1_path() {
        let (mut a, mut b) = (machine(&SPIN3), machine(&SPIN3));
        let (mut sa, mut sb) = (IdleStats::default(), IdleStats::default());
        assert_eq!(run_idle_skip_v2(&mut a, 100_001, &mut sa), Stop::Budget);
        assert_eq!(b.run_idle_skip(100_001, &mut sb), Stop::Budget);
        assert_eq!((a.cpu.r, a.cpu.insns), (b.cpu.r, b.cpu.insns));
        assert_eq!(a.state_hash(), b.state_hash());
        assert!(sa.skipped > 99_000, "{sa:?}");
        // the skip is whole periods and leaves less than one period to execute afterwards
        let after_skip = sa.executed - sa.probe_executed - MIN_SLICE;
        assert_eq!((sa.skipped % 3, after_skip < 3), (0, true), "{sa:?}");
    }

    #[test]
    fn the_probe_finds_the_period_of_a_spin_loop() {
        let mut m = machine(&SPIN3);
        m.run(10, 0, None); // reach the loop's steady state (the first `cmp` sets Z)
        assert_eq!(probe_idle_v2(&mut m, 100), (Some(3), None));
        assert!(!m.bus.probing, "the probe flag is restored");
    }

    #[test]
    fn the_state_compare_runs_once_per_visit_of_the_start_pc_not_per_instruction() {
        let mut m = machine(&SPIN3);
        m.run(10, 0, None);
        let mut c = V2Counters::default();
        assert_eq!(probe_idle_v2_counted(&mut m, 100, &mut c), (Some(3), None));
        // v1 compares after each of the 3 instructions; v2 only when the pc is back home
        assert_eq!(
            c,
            V2Counters {
                compares: 1,
                page_checks: 1
            }
        );
    }

    #[test]
    fn the_slice_outgrows_v1s_ceiling_and_never_passes_the_tick_end() {
        // loop: add r0,r0,#1 ; b loop  (never idle: every probe fails)
        let code = [0xE280_0001, 0xEAFF_FFFD];
        let (mut a, mut b) = (machine(&code), machine(&code));
        let (mut sa, mut sb) = (IdleStats::default(), IdleStats::default());
        let mut c = V2Counters::default();
        assert_eq!(
            run_idle_skip_v2_counted(&mut a, 400_000, &mut sa, &mut c),
            Stop::Budget
        );
        assert_eq!(b.run_idle_skip(400_000, &mut sb), Stop::Budget);
        assert_eq!((a.cpu.insns, a.cpu.r), (400_000, b.cpu.r));
        assert_eq!(a.reg(0), 200_000);
        assert_eq!((sa.skipped, sb.skipped), (0, 0));
        assert!(sb.probes > 30 && sa.probes * 2 < sb.probes, "{sa:?} {sb:?}");
        // the compare counter is the one the v1 loop would have paid on every probed instruction
        assert!(c.compares * 2 < sb.probe_executed, "{c:?} {sb:?}");
    }

    #[test]
    fn a_zero_budget_runs_nothing() {
        let mut m = machine(&SPIN3);
        let mut st = IdleStats::default();
        assert_eq!(run_idle_skip_v2(&mut m, 0, &mut st), Stop::Budget);
        assert_eq!((m.cpu.insns, st), (0, IdleStats::default()));
    }
}
