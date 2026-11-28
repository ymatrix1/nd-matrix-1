//! OWNER: W3-LOOP. Idle skipping v2, stage 2: the trace loop-watch, and the v2 tick (skeleton:
//! the tick of `Machine::run_tick_idle` built from the stage-1 functions).
//!
//! Spec (design note 5, WP W3-LOOP): arm when one trace self-links four times in a row or the
//! same entry pc is dispatched twice; take `S0` at that dispatch point with `bus.probing = true`;
//! compare at dispatch points and at `LOOP_BACK` where `pc == S0.pc`; skip `k * L` by the exact
//! rule of `machine/idle.rs`; disarm with back-off on impurity or after `MAX_PERIOD`
//! instructions. The instants inspected are a subset of the instants the interpreter inspects, so
//! a missed loop costs speed only. The state is `SbState::watch`; the `Watch` exit of the driver
//! returns here. `run_tick_idle_v2` combines stage 1 and stage 2: `insns` of virtual time, then
//! (on `Stop::Budget`) zero the low memory and inject the timer interrupt, exactly as
//! `Machine::run_tick_idle` does.
//!
//! The seam forwards `Machine::run_tick_idle` here when `cfg.idle_v2`; the skeleton therefore
//! must not call `run_tick_idle` (it would recurse).

use super::idle::run_idle_skip_v2;
use super::SbState;
use crate::machine::boot::TIMER_IRQ;
use crate::machine::idle::IdleStats;
use crate::machine::{Machine, Stop, LOW_MEMORY};

/// One tick with idle skipping v2: `insns` of virtual time, then the timer interrupt.
pub fn run_tick_idle_v2(
    m: &mut Machine,
    sb: &mut SbState,
    insns: u64,
    stats: &mut IdleStats,
) -> Stop {
    let _ = sb; // stage 2 (the loop-watch) lands here
    let stop = run_idle_skip_v2(m, insns, stats);
    if matches!(stop, Stop::Budget) {
        m.bus.mem.zero(0, LOW_MEMORY as usize);
        m.inject_irq(TIMER_IRQ);
    }
    stop
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
    fn the_skeleton_tick_equals_the_v1_tick_on_wfi_and_on_a_spin_loop() {
        for code in [
            &[0xEE07_0F90u32, 0xE281_1001, 0xEAFF_FFFE][..], // wfi ; add r1,r1,#1 ; b .
            &[0xE351_0000, 0x01A0_2002, 0xEAFF_FFFC][..],    // spin loop
        ] {
            let (mut a, mut b) = (machine(code), machine(code));
            let (mut sa, mut sb) = (IdleStats::default(), IdleStats::default());
            let ra = run_tick_idle_v2(&mut a, &mut SbState::default(), 10_000, &mut sa);
            let rb = b.run_tick_idle(10_000, &mut sb);
            assert_eq!((ra, sa), (rb, sb));
            assert_eq!(
                (a.wfi, a.cpu.insns, a.state_hash()),
                (b.wfi, b.cpu.insns, b.state_hash())
            );
        }
    }
}
