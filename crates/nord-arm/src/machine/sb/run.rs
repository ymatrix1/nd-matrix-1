//! OWNER: W2-DRV. The tier driver: `run_fast` (the tier-1 loop plus trace dispatch) and
//! `run_sb_full` (`Machine::run` with the driver in place of the predecode loop).
//!
//! Spec (design note 4.1, WP W2-DRV): `run_fast` replaces the `run_cached` call of `Machine::run`
//! with the same contract (runs at least one instruction or returns). With the tier off
//! (`SbConfig::enabled == false`) it is `run_cached` plus the stale drain. With it on, per loop:
//!
//! | Step | Rule |
//! |---|---|
//! | leave | the budget is used up, `pc & 3 != 0`, Thumb, `pc == until`/`stop`, or a `HOOK_X` page: return `None` |
//! | watch | `SbState::watch.armed` and `pc == watch.pc` after at least one instruction of this call: `watch.hit = true`, return `None` (the executor runs with `watch = true`, so a loop returns here after every lap) |
//! | resolve | `chain::resolve`: the linked successor of the exit just taken (`Exit::link`, or the two-way inline cache of `Indirect`/`IndirectArm`/`Dynamic` exits) when it is live, else `SbCache::lookup`; a looked-up successor patches the link |
//! | enter | only when `n_guest <= left` and no page of the trace holds `until` or `stop` (I7); `backend.run` when compiled (and no watch), else `exec::run_sb` |
//! | after a run | whatever the exit kind: take `cpu.fault`, `drain_stale` (I11: a chain never continues into a trace that just died), then the flipped re-layout check; `Stale` ends the chain, the others link |
//! | build | no trace at `pc`, `hot.bump(pc) >= cfg.hot`, `builds < cfg.builds_per_run`, the page is not volatile and the build did not fail before: `build::build_at` (marks every page as code, caps the cache, compiles, validates) |
//! | tier 1 | otherwise a slice `run_cached(min(limit, insns + cfg.slice))`, then `drain_stale` |
//!
//! Hotness is sampled at dispatch points (slice boundaries and trace exits), so a loop of any
//! period is found. A trace may start mid-block: traces depend only on code words (I1).
//!
//! # Watch contract with W3-LOOP
//!
//! The executor leaves a `LOOP_BACK` through its own exit (kind `ExitKind::LoopBack`, target the
//! loop head); no builder emits `ExitKind::Watch`. The driver turns that into the watch return at
//! the next dispatch point. The check needs one instruction of progress in the current call:
//! arming at pc `P` and calling `run_fast` at `P` must run the loop once, not return at once
//! (`watch_tests.rs`, the interpreter as oracle). `run_sb_full` ends a call with `Stop::Budget`
//! when it sees `watch.hit`; only an armed caller can observe that.
//!
//! # Stale pages
//!
//! `drain_stale` forwards `Mem::stale_code` to `SbState::on_stale` before `Machine::drop_stale_code`
//! pops it, before the first dispatch and after every tier-1 slice and trace run; `run_sb_full`
//! also drains after a hook and a plain step. While the seam holds `SbState` taken out of the
//! machine (`SbState::take_real`), `Machine::drop_stale_code` notifies the placeholder left in
//! `Machine::sb`, so the seam adds `sb.absorb_pending(&mut m.sb, &mut m.bus.mem)` as the first
//! line of `drain_stale` (W4-SEAM; the skeleton has no field to read).
//!
//! # Mutation sites (L5, ids `600..=606` of the block shared with W1-CACHE)
//!
//! [`MUTANTS`] lists them; `mutant_tests.rs` kills each one with this package's own tests.

use super::SbState;
use crate::cpu::Fault;
use crate::machine::{Machine, Stop, HOOK_SPIN};
use crate::mem::HOOK_X;

mod build;
mod chain;
#[cfg(test)]
mod cycle_tests;
#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod idle_tests;
#[cfg(test)]
mod mutant_tests;
#[cfg(test)]
mod plumbing_tests;
#[cfg(test)]
mod watch_tests;

/// `(id, name)` of the mutation sites of the driver (block `600..=605`; W1-CACHE owns `650..=655`).
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (600, "run_fast: the budget test is skipped"),
    (601, "run_fast: the until/stop page test is skipped"),
    (602, "run_fast: no stale drain after a trace run"),
    (603, "run_fast: a dead link is followed"),
    (
        604,
        "run_fast: the pages of a built trace are not marked as code",
    ),
    (605, "run_fast: the fault of a trace run is dropped"),
    (606, "run_fast: a looked-up successor is not linked"),
];

/// Forward every page that went stale since the last call to `sb`, then drop the decoded pages
/// (`Machine::drop_stale_code`). Allocation-free.
pub fn drain_stale(m: &mut Machine, sb: &mut SbState) {
    for i in 0..m.bus.mem.stale_code.len() {
        let page = m.bus.mem.stale_code[i];
        sb.on_stale(page, &mut m.bus.mem);
    }
    m.drop_stale_code();
}

/// True when the pc cannot be dispatched here: the budget is used up, a misaligned or Thumb pc,
/// the `until`/`stop` addresses or a `HOOK_X` page. Same tests as the loop of `Machine::run`.
fn must_leave(m: &Machine, limit: u64, until: u32, stop: u32) -> bool {
    let pc = m.cpu.r[15];
    m.cpu.insns >= limit
        || pc & 3 != 0
        || m.cpu.thumb
        || pc == until
        || pc == stop
        || m.bus.mem.flags(pc) & HOOK_X != 0
}

/// Run from the current pc until the budget `limit` (a value of `cpu.insns`) is used up, `pc`
/// reaches `until`/`stop`, the code leaves the tiers (Thumb, hooked or misaligned pc), the watch
/// returns or an instruction faults. Same contract as `Machine::run_cached`.
pub fn run_fast(
    m: &mut Machine,
    sb: &mut SbState,
    limit: u64,
    until: u32,
    stop: u32,
) -> Option<Fault> {
    drain_stale(m, sb);
    if !sb.cfg.enabled {
        let fault = m.run_cached(limit, until, stop);
        drain_stale(m, sb);
        return fault;
    }
    let start = m.cpu.insns;
    let mut builds = 0u8;
    let mut from = None;
    loop {
        if must_leave(m, limit, until, stop) {
            return None;
        }
        let pc = m.cpu.r[15];
        if sb.watch.armed && pc == sb.watch.pc && m.cpu.insns > start {
            sb.watch.hit = true;
            return None;
        }
        let left = limit - m.cpu.insns;
        match chain::resolve(sb, from.take(), pc) {
            Some(id) => {
                if chain::enterable(sb.cache.get(id), left, until, stop) {
                    match chain::enter(m, sb, id, limit) {
                        Ok(leave) => from = leave,
                        Err(fault) => return Some(fault),
                    }
                    continue;
                }
            }
            None => {
                if builds < sb.cfg.builds_per_run
                    && !sb.cache.volatile(pc >> 12)
                    && !sb.hot.failed(pc)
                    && sb.hot.bump(pc) >= sb.cfg.hot
                {
                    builds += 1;
                    if build::build_at(m, sb, pc) {
                        continue;
                    }
                }
            }
        }
        let before = m.cpu.insns;
        let upto = limit.min(before + u64::from(sb.cfg.slice.max(1)));
        let fault = m.run_cached(upto, until, stop);
        sb.stats.guest_in_tier1 += m.cpu.insns - before;
        drain_stale(m, sb);
        if fault.is_some() {
            return fault;
        }
    }
}

/// [`Machine::run`] with the tier driver in place of the predecode loop (the outer loop is a
/// copy of `Machine::run`: hooks, `until`/`stop`, budget, plain step). Entry for tests and tools
/// before the seam; the equivalence oracle after it.
pub fn run_sb_full(
    m: &mut Machine,
    sb: &mut SbState,
    max: u64,
    until: u32,
    stop_pc: Option<u32>,
) -> (u64, Stop) {
    let start = m.cpu.insns;
    let limit = start + max;
    drain_stale(m, sb);
    let mut page = u32::MAX;
    let mut hooked = false;
    let stop = stop_pc.unwrap_or(u32::MAX);
    let mut spin = 0u32;
    let reason = loop {
        if m.cpu.insns >= limit {
            break Stop::Budget;
        }
        let pc = m.cpu.r[15];
        if pc == until {
            break Stop::Until;
        }
        if pc == stop {
            break Stop::StopPc;
        }
        if pc >> 12 != page {
            page = pc >> 12;
            hooked = m.bus.mem.flags(pc) & HOOK_X != 0;
        }
        if hooked && m.run_hook(pc) {
            m.stats.hook_runs += 1;
            drain_stale(m, sb);
            page = u32::MAX;
            spin += 1;
            if spin >= HOOK_SPIN {
                break Stop::HookLoop(m.cpu.r[15]);
            }
            continue;
        }
        spin = 0;
        if m.predecode && !hooked && !m.cpu.thumb && pc & 3 == 0 {
            if let Some(f) = run_fast(m, sb, limit, until, stop) {
                break Stop::Fault(f);
            }
            if sb.watch.hit {
                break Stop::Budget;
            }
            continue;
        }
        m.bus.pc = pc;
        m.stats.plain_steps += 1;
        let stepped = m.cpu.step(&mut m.bus);
        drain_stale(m, sb);
        if let Err(f) = stepped {
            break Stop::Fault(f);
        }
    };
    (m.cpu.insns - start, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::SbConfig;

    const CODE: u32 = 0xC000_0000;

    fn machine(code: &[u32]) -> Machine {
        let mut m = Machine::new();
        for (i, w) in code.iter().enumerate() {
            m.bus.mem.write32(CODE + 4 * i as u32, *w);
        }
        m.cpu.r[15] = CODE;
        m
    }

    #[test]
    fn run_fast_runs_the_cached_loop_to_the_budget() {
        // add r0,r0,#1 ; add r0,r0,#1 ; add r0,r0,#1 ; b .
        let code = [0xE280_0001, 0xE280_0001, 0xE280_0001, 0xEAFF_FFFE];
        let mut m = machine(&code);
        let mut sb = SbState {
            cfg: SbConfig::on(),
            ..SbState::default()
        };
        assert_eq!(run_fast(&mut m, &mut sb, 3, 0, u32::MAX), None);
        assert_eq!((m.cpu.r[0], m.cpu.insns, m.cpu.r[15]), (3, 3, CODE + 12));
    }

    #[test]
    fn a_store_into_decoded_code_is_forwarded_to_the_state_and_drained() {
        // str r1,[r2] (r2 -> own page) ; b .
        let mut m = machine(&[0xE582_1000, 0xEAFF_FFFE]);
        m.cpu.r[2] = CODE + 0x100;
        let mut sb = SbState::default();
        let id = sb
            .cache
            .insert(tiny_trace(CODE + 0x800, &[(CODE + 0x800, 7)]));
        assert!(sb.cache.is_live(id));
        run_fast(&mut m, &mut sb, 1, 0, u32::MAX);
        assert!(
            !sb.cache.is_live(id),
            "the store made the page stale: its trace died"
        );
        assert!(m.bus.mem.stale_code.is_empty(), "and the queue is drained");
        assert_eq!(sb.stats.dropped, 1);
    }

    #[test]
    fn drain_stale_drops_the_decoded_page_too() {
        let mut m = machine(&[0xE280_0001, 0xEAFF_FFFE]);
        m.run(1, 0, None);
        assert_eq!(m.code.live_pages(), 1);
        m.bus.mem.write32(CODE + 4, 0xEAFF_FFFE); // store to the decoded page
        assert_eq!(m.bus.mem.stale_code.len(), 1);
        drain_stale(&mut m, &mut SbState::default());
        assert_eq!((m.code.live_pages(), m.bus.mem.stale_code.len()), (0, 0));
    }

    #[test]
    fn run_sb_full_equals_run_on_a_program_with_until_and_stop() {
        let code = [
            0xE3A0_0000,
            0xE280_0001,
            0xE350_000A,
            0x1AFF_FFFC,
            0xEAFF_FFFE,
        ];
        for (max, until, stop) in [
            (100, 0, None),
            (100, CODE + 12, None),
            (100, 0, Some(CODE + 16)),
            (5, 0, None),
        ] {
            let (mut a, mut b) = (machine(&code), machine(&code));
            let ra = a.run(max, until, stop);
            let rb = run_sb_full(&mut b, &mut SbState::default(), max, until, stop);
            assert_eq!(ra, rb, "{max} {until:x} {stop:x?}");
            assert_eq!((a.cpu.r, a.cpu.insns), (b.cpu.r, b.cpu.insns));
        }
    }

    #[test]
    fn the_same_program_with_the_tier_on_equals_the_interpreter_and_enters_traces() {
        let code = [
            0xE3A0_0000,
            0xE280_0001,
            0xE350_000A,
            0x1AFF_FFFC,
            0xEAFF_FFFE,
        ];
        let mut entered = 0;
        for (max, until, stop) in [(100, 0, None), (100, CODE + 16, None), (7, 0, None)] {
            let (mut a, mut b) = (machine(&code), machine(&code));
            let mut sb = SbState {
                cfg: SbConfig::on(),
                ..SbState::default()
            };
            let ra = a.run(max, until, stop);
            let rb = run_sb_full(&mut b, &mut sb, max, until, stop);
            assert_eq!(ra, rb, "{max} {until:x} {stop:x?}");
            assert_eq!((a.cpu.r, a.cpu.insns), (b.cpu.r, b.cpu.insns));
            assert_eq!(a.cpu.nzcv(), b.cpu.nzcv());
            entered += sb.stats.enters;
        }
        assert!(entered > 0, "the loop ran in traces");
    }
}
