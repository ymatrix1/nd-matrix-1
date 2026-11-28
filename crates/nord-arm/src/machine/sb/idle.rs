//! OWNER: W1-IDLE. Idle skipping v2, stage 1: tier 1 only, exact by construction (skeleton:
//! the v1 probe and skip).
//!
//! Spec (design note 5, WP W1-IDLE): `probe_idle_v2` calls `same_state` only when `r[15]`
//! equals the start pc (a necessary condition, so the first matching instant is unchanged),
//! hoists the `HOOK_X` and `stop_pc` checks to page changes, and after a failed probe grows the
//! next slice geometrically past `MAX_SLICE` (up to 2^20, never past the tick end). The skip
//! rule `k * L` is untouched. Reuses `machine::idle::same_state` and `Machine::probe_loop` (made
//! `pub(super)` by the skeleton; never copy them) and the `pub(super)` items of `machine::probe`.
//! The tests compare the final state with `run_idle_skip` on the idle tests of `machine/idle.rs`
//! and on random programs with spin loops of period 1..40.

use crate::machine::idle::IdleStats;
use crate::machine::{Machine, Stop};

/// Execute up to `MAX_PERIOD` instructions looking for a pure loop (contract of
/// `Machine::probe_idle`): the loop period when found, else `None` plus an optional stop.
pub fn probe_idle_v2(m: &mut Machine, limit: u64) -> (Option<u64>, Option<Stop>) {
    m.probe_idle(limit)
}

/// Run `budget` instructions of virtual time, skipping provably idle loops (contract of
/// `Machine::run_idle_skip`).
pub fn run_idle_skip_v2(m: &mut Machine, budget: u64, stats: &mut IdleStats) -> Stop {
    m.run_idle_skip(budget, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(code: &[u32]) -> Machine {
        let mut m = Machine::new();
        for (i, w) in code.iter().enumerate() {
            m.bus.mem.write32(0xC000_0000 + 4 * i as u32, *w);
        }
        m.cpu.r[15] = 0xC000_0000;
        m
    }

    #[test]
    fn the_skeleton_skips_a_pure_loop_exactly_like_the_v1_path() {
        // loop: cmp r1,#0 ; moveq r2,r2 ; b loop
        let code = [0xE351_0000, 0x01A0_2002, 0xEAFF_FFFC];
        let (mut a, mut b) = (machine(&code), machine(&code));
        let (mut sa, mut sb) = (IdleStats::default(), IdleStats::default());
        assert_eq!(run_idle_skip_v2(&mut a, 100_001, &mut sa), Stop::Budget);
        assert_eq!(b.run_idle_skip(100_001, &mut sb), Stop::Budget);
        assert_eq!((sa, a.cpu.r, a.cpu.insns), (sb, b.cpu.r, b.cpu.insns));
        assert!(sa.skipped > 99_000, "{sa:?}");
    }

    #[test]
    fn the_skeleton_probe_finds_the_period_of_a_spin_loop() {
        let mut m = machine(&[0xE351_0000, 0x01A0_2002, 0xEAFF_FFFC]);
        m.run(10, 0, None); // reach the loop's steady state (the first `cmp` sets Z)
        assert_eq!(probe_idle_v2(&mut m, 100), (Some(3), None));
    }
}
