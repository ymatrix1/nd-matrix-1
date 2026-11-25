//! Idle skipping: fast-forward spin loops that provably change nothing.
//!
//! **Rule (exact, no firmware knowledge).** Execute from the current state `S0` with an
//! instrumented bus. If the CPU returns to the same `pc` with the same registers and CPSR after
//! `L` instructions, and during those `L` instructions
//!
//! * no store changed a byte of memory (a store of the value already there is allowed),
//! * no load or store touched a device page (tick, poll windows, GPIO, DSP ports),
//! * no code hook (stub, null guard) ran, and no unmapped page was mapped,
//!
//! then the machine state after the iteration equals `S0`, so every further iteration is the same
//! and a run of `n` more instructions ends in `S0` advanced by `n mod L` instructions. The skip
//! advances the instruction counter by `k * L` without executing. External events (the timer
//! interrupt injected at a tick boundary) are not part of the loop and still run for real.
//!
//! `WFI` (`mcr p15, 0, rd, c7, c0, 4`) is the degenerate case: the rest of the tick is skipped.
//! Equivalence is checked by `tests` and by the per-tick state hash of `arm-lle-bench`.

use super::{Machine, Stop};
use crate::cpu::{Cpu, Fault};
use crate::mem::HOOK_X;

/// Longest loop body the probe looks for (instructions).
pub const MAX_PERIOD: u64 = 4096;
/// First slice of normal execution between probes; doubles after each failed probe.
pub const MIN_SLICE: u64 = 64;
pub const MAX_SLICE: u64 = 8192;

/// Counters for reports and tests.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleStats {
    pub probes: u64,
    pub skips: u64,
    /// Instructions advanced without executing.
    pub skipped: u64,
    /// Instructions executed (probe instructions included).
    pub executed: u64,
    /// Part of `executed` that ran inside probes (the plain, instrumented path).
    pub probe_executed: u64,
    pub wfi: u64,
}

fn same_state(a: &Cpu, b: &Cpu) -> bool {
    a.r == b.r && a.cpsr() == b.cpsr() && a.spsr == b.spsr && a.cp15_ctrl == b.cp15_ctrl
}

impl Machine {
    /// Execute up to `MAX_PERIOD` instructions looking for a pure loop. Returns the loop period
    /// when found (the CPU is back in the starting state), else `None` (the executed instructions
    /// are real progress). The bus reports side effects while `probing` (see `probe.rs`).
    pub fn probe_idle(&mut self, limit: u64) -> (Option<u64>, Option<Stop>) {
        let start = self.cpu.clone();
        self.bus.probing = true;
        self.bus.impure = false;
        let out = self.probe_loop(&start, limit.min(MAX_PERIOD));
        self.bus.probing = false;
        out
    }

    fn probe_loop(&mut self, start: &Cpu, n: u64) -> (Option<u64>, Option<Stop>) {
        for i in 1..=n {
            let pc = self.cpu.r[15];
            if Some(pc) == self.stop_pc {
                return (None, Some(Stop::StopPc));
            }
            if self.bus.mem.flags(pc) & HOOK_X != 0 {
                return (None, None);
            }
            self.bus.pc = pc;
            let fault = if self.predecode && !self.cpu.thumb && pc & 3 == 0 {
                self.step_cached(pc)
            } else {
                self.cpu.step(&mut self.bus).err()
            };
            if let Some(f) = fault {
                return (None, Some(Stop::Fault(f)));
            }
            if self.bus.impure {
                return (None, None);
            }
            if same_state(&self.cpu, start) {
                return (Some(i), None);
            }
        }
        (None, None)
    }

    /// Run `budget` instructions of virtual time, skipping provably idle loops. Returns the stop
    /// reason (`Budget` when the whole budget elapsed). `stats` accumulates counters.
    pub fn run_idle_skip(&mut self, budget: u64, stats: &mut IdleStats) -> Stop {
        let end = self.cpu.insns + budget;
        let mut slice = MIN_SLICE;
        while self.cpu.insns < end {
            let remaining = end - self.cpu.insns;
            let before = self.cpu.insns;
            let (_, stop) = self.run(slice.min(remaining), super::SENTINEL, self.stop_pc);
            stats.executed += self.cpu.insns - before;
            match stop {
                Stop::Budget => {}
                Stop::Fault(Fault::Wfi) => {
                    stats.wfi += 1;
                    self.wfi += 1;
                    stats.skipped += end - self.cpu.insns;
                    self.cpu.insns = end;
                    return Stop::Budget;
                }
                other => return other,
            }
            let remaining = end - self.cpu.insns;
            if remaining == 0 {
                break;
            }
            stats.probes += 1;
            let before = self.cpu.insns;
            let (period, stop) = self.probe_idle(remaining);
            stats.executed += self.cpu.insns - before;
            stats.probe_executed += self.cpu.insns - before;
            if let Some(Stop::Fault(Fault::Wfi)) = stop {
                stats.wfi += 1;
                self.wfi += 1;
                stats.skipped += end - self.cpu.insns;
                self.cpu.insns = end;
                return Stop::Budget;
            }
            if let Some(s) = stop {
                return s;
            }
            match period {
                Some(l) => {
                    let k = (end - self.cpu.insns) / l;
                    self.cpu.insns += k * l;
                    stats.skips += 1;
                    stats.skipped += k * l;
                    slice = MIN_SLICE;
                }
                None => slice = (slice * 2).min(MAX_SLICE),
            }
        }
        Stop::Budget
    }

    /// One tick with idle skipping: `insns` of virtual time, then the timer interrupt.
    pub fn run_tick_idle(&mut self, insns: u64, stats: &mut IdleStats) -> Stop {
        let stop = self.run_idle_skip(insns, stats);
        if matches!(stop, Stop::Budget) {
            self.bus.mem.zero(0, super::LOW_MEMORY as usize);
            self.inject_irq(super::boot::TIMER_IRQ);
        }
        stop
    }
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
    fn pure_loop_is_skipped_and_state_matches_a_full_run() {
        // ldr-free spin: cmp r0,r0 ; beq . ... use: loop: cmp r1,#0 ; moveq r2,r2 ; b loop (3 insns)
        let code = [0xE351_0000, 0x01A0_2002, 0xEAFF_FFFC];
        let mut a = machine(&code);
        let mut b = machine(&code);
        let mut st = IdleStats::default();
        let s = a.run_idle_skip(100_001, &mut st);
        assert_eq!(s, Stop::Budget);
        b.run(100_001, 0, None);
        assert_eq!(a.cpu.insns, b.cpu.insns);
        assert!(a.cpu.r == b.cpu.r && a.cpu.cpsr() == b.cpu.cpsr());
        assert!(st.skips >= 1 && st.skipped > 99_000, "{st:?}");
        assert!(st.executed < 1_000, "{st:?}");
    }

    #[test]
    fn counting_loop_is_not_skipped_because_registers_change() {
        // loop: add r0,r0,#1 ; b loop
        let mut m = machine(&[0xE280_0001, 0xEAFF_FFFD]);
        let mut st = IdleStats::default();
        m.run_idle_skip(10_000, &mut st);
        assert_eq!(st.skipped, 0);
        assert_eq!(m.reg(0), 5_000);
    }

    #[test]
    fn loop_that_changes_memory_is_not_skipped_but_idempotent_store_is() {
        // r1 = 0x2000 ; loop: str r0,[r1] ; b loop   (r0 = 0 over zero memory: idempotent)
        let mut m = machine(&[0xE3A0_1A02, 0xE581_0000, 0xEAFF_FFFD]);
        let mut st = IdleStats::default();
        m.run_idle_skip(10_000, &mut st);
        assert!(st.skipped > 9_000, "{st:?}");
        // loop that stores a counter: ldr r0,[r1] ; add r0,r0,#1 ; str r0,[r1] ; b loop
        let mut m = machine(&[
            0xE3A0_1A02,
            0xE591_0000,
            0xE280_0001,
            0xE581_0000,
            0xEAFF_FFFB,
        ]);
        let mut st = IdleStats::default();
        m.run_idle_skip(10_000, &mut st);
        assert_eq!(st.skipped, 0);
        assert!(m.bus.mem.read32(0x2000) > 2_000);
    }

    #[test]
    fn device_polling_loop_is_never_skipped() {
        // r1 = 0x6000_0000 ; loop: ldr r0,[r1] ; b loop  (poll window read is a device access)
        let mut m = machine(&[0xE3A0_1206, 0xE591_0000, 0xEAFF_FFFD]);
        let mut st = IdleStats::default();
        m.run_idle_skip(5_000, &mut st);
        assert_eq!(st.skipped, 0);
        assert!(m.bus.device_reads > 1_000);
    }

    #[test]
    fn wfi_skips_the_rest_of_the_tick() {
        // mcr p15,0,r0,c7,c0,4 ; b .
        let mut m = machine(&[0xEE07_0F90, 0xEAFF_FFFE]);
        let mut st = IdleStats::default();
        assert_eq!(m.run_idle_skip(200_000, &mut st), Stop::Budget);
        assert_eq!((st.wfi, st.skipped), (1, 199_999));
        assert_eq!(m.cpu.insns, 200_000);
    }

    #[test]
    fn wfi_has_one_semantic_in_the_full_and_the_skipping_path() {
        // mcr p15,0,r0,c7,c0,4 ; add r1,r1,#1 ; b .   (the add runs once, then spins)
        let code = [0xEE07_0F90, 0xE281_1001, 0xEAFF_FFFE];
        let (mut a, mut b) = (machine(&code), machine(&code));
        let mut st = IdleStats::default();
        assert_eq!(a.run_tick_idle(10_000, &mut st), Stop::Budget);
        assert_eq!(b.run_tick(10_000), Stop::Budget);
        assert_eq!((a.wfi, b.wfi), (1, 1));
        assert_eq!(a.state_hash(), b.state_hash());
        assert_eq!((a.cpu.insns, a.reg(1)), (b.cpu.insns, b.reg(1)));
    }
}
