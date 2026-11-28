//! OWNER: W3-NINT. `impl Backend for NativeBackend` (skeleton: native code is unavailable, so
//! there is no backend and the driver stays on the T2 executor).
//!
//! Spec (design note 9, WP W3-NINT): `new()` is `Some` only when `native::available()`; it wraps
//! one `rt::Jit` (W1-NCORE; 32 MB cap, rebuilt after a pass boundary on overflow), the link table
//! and the compiled functions. `compile` builds the IR with `codegen::build_function`, defines it
//! in the module and returns the id, or `None` (the trace stays on T2: never an error). `run`
//! refreshes `NativeCtl` from `Mem::raw_parts`, calls `rt::call`, applies the exit exactly as
//! `exec::run_sb` does (`left -= done`, `bus.pc = last_pc`) and returns the `RunOut`; an
//! `EXIT_GENERIC` return retires what native code counted, runs one `Cpu::step` through the
//! `SysBus` and reports `backend::EXIT_STEPPED` (the driver, not the backend, drains stale after
//! every run). While `bus.probing` is set, `run` reports `backend::EXIT_SLOW` without running.
//! Native code never runs while `bus.probing` is set.

use super::available;
use crate::cpu::Cpu;
use crate::machine::sb::backend::{Backend, RunOut};
use crate::machine::sb::uop::Sb;
use crate::machine::SysBus;

/// The tier-3 backend.
#[derive(Debug)]
pub struct NativeBackend {
    _private: (),
}

impl NativeBackend {
    /// The backend, or `None` when native code cannot run here (the caller keeps T2).
    pub fn new() -> Option<Self> {
        available().then_some(Self { _private: () })
    }
}

impl Backend for NativeBackend {
    fn compile(&mut self, sb: &Sb) -> Option<u32> {
        let _ = sb;
        None
    }

    fn release(&mut self, id: u32) {
        let _ = id;
    }

    fn name(&self) -> &'static str {
        "native"
    }

    fn run(&mut self, id: u32, cpu: &mut Cpu, bus: &mut SysBus, left: &mut u64) -> RunOut {
        let _ = (id, cpu, bus, left);
        unreachable!("no trace is ever compiled by the skeleton (OWNER W3-NINT)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;

    #[test]
    fn there_is_no_backend_while_native_code_is_unavailable() {
        assert_eq!(NativeBackend::new().is_some(), available());
        assert!(!available(), "the skeleton never claims native support");
    }

    #[test]
    fn the_skeleton_compiles_nothing_and_names_itself() {
        let mut b = NativeBackend { _private: () };
        assert_eq!(b.name(), "native");
        assert_eq!(b.compile(&tiny_trace(0x1000, &[(0x1000, 1)])), None);
        b.release(0);
    }

    #[test]
    #[should_panic(expected = "W3-NINT")]
    fn running_a_trace_that_was_never_compiled_is_a_bug() {
        let mut b = NativeBackend { _private: () };
        b.run(0, &mut Cpu::new(), &mut SysBus::new(), &mut 1);
    }
}
