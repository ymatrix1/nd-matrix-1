//! Handlers of the core range `0x000..0x100`: `NOP`, `INTERP`, `FIN`, `BAIL`.
//!
//! OWNER: W2-DRIVE. `INTERP` looks the instruction up at `u.pc` (`decode_cache.get`, then
//! `fetch_and_cache`, `u.a` is the ISA flag), runs `exec_cached` and then re-checks the deadline
//! (`irq.due`) before the next uop returns `Ok`; `FIN` runs `finish_step`; `NOP` only accounts
//! its steps.
//!
//! Rules the package must implement (design 11.1, review of 2026-11-28; each one has a test):
//! 1. `INTERP` materializes first (`core.pc = u.pc`, `core.steps = base_steps + done`), because
//!    `exec_cached` and the system-register paths read both (timers count `steps`).
//! 2. After `exec_cached` the core is already at the next boundary. If `core.pc != u.b` (a flow
//!    instruction, a loop back-edge or a resolved delayed branch moved it) the handler returns
//!    `Exit::Jump` with `ctx.next_pc = core.pc`; a block never continues past such a uop.
//! 3. It then checks `uop_run.flags & RF_SMC` (an interpreted store may hit translated code) and
//!    returns `Exit::Smc`, and re-checks `irq.due` against the steps still owed by the block
//!    (`Exit::Slow`), exactly as design 4.5 rule 4 says.
//! 4. It adds 1 to `uop_run.stats.steps_interp_uop` (gate G2 reads it; the driver counts only
//!    `steps_uop`). An `Err(Fault)` from `exec_cached` is stored in `uop_run.fault` and returned
//!    as `Exit::Fault` with the partial effects the interpreter left: no replay.
//!
//! Skeleton: only `BAIL` is registered, so every other kind keeps the default handler and asks
//! the driver for the interpreter.
use super::super::types::{bail_handler, Handler, UopKind};

/// Fills the core range of `t`.
pub(crate) fn register(t: &mut [Handler]) {
    t[usize::from(UopKind::BAIL.0)] = bail_handler;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::{KindFamily, Uop, UopCtx};

    /// Distinct body so the linker cannot fold it with `bail_handler`.
    unsafe extern "C" fn marker_handler(_: *mut UopCtx, _: *const Uop) -> u32 {
        0xFEED
    }

    /// Compares handler pointers only against a copy of the same value (function addresses of
    /// separately named items are not guaranteed to be identical across codegen units).
    #[test]
    fn registration_writes_only_the_core_range() {
        let marker: Handler = marker_handler;
        let mut t = vec![marker; usize::from(UopKind::MAX)];
        register(&mut t);
        let bail = usize::from(UopKind::BAIL.0);
        assert!(KindFamily::Core.range().contains(&bail));
        for (i, h) in t.iter().enumerate() {
            let untouched = *h as usize == marker as usize;
            assert_eq!(untouched, i != bail, "slot {i:#x}");
        }
    }
}
