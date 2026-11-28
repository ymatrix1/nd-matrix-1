//! OWNER: S0 (frozen). The execution backend behind a compiled trace: the contract tier 3
//! (native code) implements, plus [`Tier2Backend`], the in-process executor behind the same
//! trait (the oracle tier 3 is compared against, and the plumbing test of the driver).

use super::exec;
use super::uop::Sb;
use crate::cpu::Cpu;
use crate::machine::SysBus;

/// `RunOut::exit` value of a run that did not execute the trace: the native entry could not run
/// it (disabled, a guard of the trace failed to hold at entry, or the backend gave up). Nothing
/// was applied (`cpu`, `left`, `bus.pc` untouched), so the driver runs the trace with
/// `exec::run_sb` instead. Never a valid index into `Sb::exits` (at most `uop::MAX_EXITS`).
pub const EXIT_SLOW: u16 = u16::MAX;

/// `RunOut::exit` value of a run that ended in the interpreter: native code left with the state
/// exact before an instruction it does not handle (`native::abi::EXIT_GENERIC`), the backend
/// retired the instructions before it (`left`, `cpu.insns`), ran that one instruction through
/// `Cpu::step` and reports `target = cpu.r[15]`. The exit is applied, there is no static exit to
/// link: the driver re-dispatches at `target` and never patches `Exit::link` for it. A fault of
/// that step stays in `cpu.fault`. Never a valid index into `Sb::exits`.
pub const EXIT_STEPPED: u16 = u16::MAX - 1;

/// What a backend run reports. The exit was already applied (see the conventions in `uop.rs`):
/// `cpu.insns`, `left`, `cpu.r[15]`, `bus.pc` and the flags are final.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunOut {
    /// Index into the trace's `exits`, [`EXIT_STEPPED`] or [`EXIT_SLOW`] (nothing ran).
    pub exit: u16,
    /// The new `cpu.r[15]` (also known to the driver for linking).
    pub target: u32,
}

/// A way to run compiled traces. Same contract as [`exec::run_sb`].
pub trait Backend {
    /// Compile `sb`; `None` leaves the trace to the T2 executor (never an error).
    fn compile(&mut self, sb: &Sb) -> Option<u32>;
    /// Forget a compiled trace.
    fn release(&mut self, id: u32);
    fn name(&self) -> &'static str;
    /// Run trace `id` (an id `compile` returned). Leaves with the state exact at the exit.
    fn run(&mut self, id: u32, cpu: &mut Cpu, bus: &mut SysBus, left: &mut u64) -> RunOut;
}

/// The T2 executor as a [`Backend`]: `compile` keeps a copy of the trace, `run` interprets it.
#[derive(Default)]
pub struct Tier2Backend {
    traces: Vec<Option<Sb>>,
}

impl Backend for Tier2Backend {
    fn compile(&mut self, sb: &Sb) -> Option<u32> {
        self.traces.push(Some(sb.clone()));
        Some(self.traces.len() as u32 - 1)
    }

    fn release(&mut self, id: u32) {
        if let Some(slot) = self.traces.get_mut(id as usize) {
            *slot = None;
        }
    }

    fn name(&self) -> &'static str {
        "tier2"
    }

    fn run(&mut self, id: u32, cpu: &mut Cpu, bus: &mut SysBus, left: &mut u64) -> RunOut {
        let sb = self.traces[id as usize]
            .as_ref()
            .expect("run of a released trace");
        exec::run_sb(cpu, bus, sb, left, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::trace::{build, BuildOpts};
    use crate::machine::Machine;

    #[test]
    fn the_slow_marker_is_never_an_exit_index() {
        let max = crate::machine::sb::uop::MAX_EXITS;
        assert!(EXIT_SLOW as usize > max && EXIT_STEPPED as usize > max);
        assert_ne!(EXIT_SLOW, EXIT_STEPPED);
    }

    #[test]
    fn the_tier2_backend_runs_a_compiled_trace_and_forgets_a_released_one() {
        let mut m = Machine::new();
        m.bus.mem.write32(0xC000_0000, 0xE3A0_002A); // mov r0,#42
        m.cpu.r[15] = 0xC000_0000;
        let sb = build(&mut m.bus.mem, 0xC000_0000, &BuildOpts::default()).unwrap();
        let mut be = Tier2Backend::default();
        let id = be.compile(&sb).unwrap();
        assert_eq!(be.name(), "tier2");
        let mut left = 10u64;
        let out = be.run(id, &mut m.cpu, &mut m.bus, &mut left);
        assert_eq!(
            (m.cpu.r[0], m.cpu.r[15], m.cpu.insns, left),
            (42, 0xC000_0004, 1, 9)
        );
        assert_eq!(out.target, 0xC000_0004);
        be.release(id);
        assert!(be.traces[id as usize].is_none());
    }
}
