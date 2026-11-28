//! OWNER: W2-DRV. The tier driver (skeleton: `run_fast` is the tier-1 loop plus the stale drain).
//!
//! Spec (design note 4.1, WP W2-DRV): `run_fast` replaces the `run_cached` call of
//! `Machine::run` with the same contract (runs at least one instruction or returns). Loop:
//! drain stale pages; at `pc` return when the budget is used up, `pc & 3 != 0`, Thumb, `pc ==
//! until`/`stop` or a `HOOK_X` page; look up a trace and enter it when `n_guest <= left` and no
//! page of it holds `until`/`stop` (I7) (`backend.run` if compiled, else `exec::run_sb`);
//! patch `Exit::link` or the two-way inline cache after the first lookup and chain only into a
//! live successor that passes the same tests; after every trace run, whatever its exit kind,
//! take `cpu.fault` and `drain_stale` (an ender run through `exec_decoded` may store, and a
//! chain must not continue into a trace that just died); a `Watch` exit returns to the idle
//! driver; a run that reports `backend::EXIT_SLOW` is repeated with `exec::run_sb`, one that
//! reports `backend::EXIT_STEPPED` is re-dispatched at its target without linking; `hot.bump` at
//! dispatch points, build with `trace::build` (at most `cfg.builds_per_run` per call), `Mem::mark_code`
//! every page, `SbCache::insert`, optional `backend.compile` and `validate::validate_trace`;
//! otherwise run a tier-1 slice `m.run_cached(limit.min(insns + cfg.slice), until, stop)` and
//! drain again. A guard exit above half of the trace's entries after 64 entries triggers one
//! rebuild with the prediction flipped. `run_sb_full` copies the outer loop of `Machine::run`
//! (hooks, `until`/`stop`, budget, plain step) and calls `run_fast` where it calls `run_cached`.
//!
//! While the seam holds `SbState` taken out of the machine (`SbState::take_real`),
//! `Machine::drop_stale_code` notifies the placeholder left in `Machine::sb`, which only records
//! the pages (`SbState::pending`). `drain_stale` therefore forwards `m.sb.pending` first with
//! `sb.absorb_pending(&mut m.sb, &mut m.bus.mem)` (the call is added by W4-SEAM together with the
//! `Machine::sb` field; the skeleton has no field to read), then the queue, and the driver drains
//! before and after every tier-1 stretch and after every trace run. The seam ends with
//! `real.put_back(&mut m.sb, &mut m.bus.mem)`, so no page is ever lost.

use super::SbState;
use crate::cpu::Fault;
use crate::machine::{Machine, Stop};

/// Forward every page that went stale since the last call to `sb`, then drop the decoded pages
/// (`Machine::drop_stale_code`). Allocation-free.
pub fn drain_stale(m: &mut Machine, sb: &mut SbState) {
    for i in 0..m.bus.mem.stale_code.len() {
        let page = m.bus.mem.stale_code[i];
        sb.on_stale(page, &mut m.bus.mem);
    }
    m.drop_stale_code();
}

/// Run from the current pc until the budget `limit` (a value of `cpu.insns`) is used up, `pc`
/// reaches `until`/`stop`, the code leaves the tiers (Thumb, hooked or misaligned pc) or an
/// instruction faults. Same contract as `Machine::run_cached`; the skeleton is that loop.
pub fn run_fast(
    m: &mut Machine,
    sb: &mut SbState,
    limit: u64,
    until: u32,
    stop: u32,
) -> Option<Fault> {
    drain_stale(m, sb);
    let fault = m.run_cached(limit, until, stop);
    drain_stale(m, sb);
    fault
}

/// [`Machine::run`] with the tier driver in place of the predecode loop (the skeleton is
/// `Machine::run` itself). Entry for tests and tools before the seam; the equivalence oracle
/// after it.
pub fn run_sb_full(
    m: &mut Machine,
    sb: &mut SbState,
    max: u64,
    until: u32,
    stop_pc: Option<u32>,
) -> (u64, Stop) {
    let _ = sb;
    m.run(max, until, stop_pc)
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
}
