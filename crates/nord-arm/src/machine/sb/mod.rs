//! OWNER: S0 (frozen). ARM0 execution tiers above the predecode cache: trace superblocks over a
//! flat micro-op IR (tier 2, safe Rust), an optional native tier over the same IR (tier 3,
//! feature `sb-native`), and exact idle skipping v2.
//!
//! Design: `docs/research/arm0-jit-design.md` (sections 3 to 9 and the skeleton contract in 11).
//! Everything here is off by default (`SbConfig::default()`); no existing code path calls into
//! this module until the seam work package (W4-SEAM) wires `Machine::run_tier`. Every
//! instruction that is not lowered runs through the interpreter's own handler (`GENERIC`), so a
//! missing lowering costs speed, never exactness (I8).
//!
//! # Map (owner of each file)
//!
//! | Area | Files | Owner |
//! |---|---|---|
//! | IR, config, counters, decode, bus, backend | `uop` `config` `stats` `decode` `bus` `backend` | S0, frozen |
//! | Lowering | `lower_dp` / `lower_mem` / `lower_flow` + `trace` | W1-DP / W1-MEM / W1-FLOW |
//! | Executor | `exec_alu` / `exec_mem` / `exec` + `exec_flow` | W1-XALU / W1-XMEM / W2-XCORE |
//! | Cache and invalidation | `cache` `hot` `stale` | W1-CACHE |
//! | Driver | `run` | W2-DRV |
//! | Peephole passes | `fuse` `pred` `flagopt` | W2-PEEP |
//! | Idle skipping v2 | `idle` / `loopwatch` | W1-IDLE / W3-LOOP |
//! | Verification | `testkit` `testgen` (W1-VGEN), `difftest` `mutants` `shadow` `validate` (W3-VERIF) | see files |
//! | Native tier | `native/*` (feature `sb-native`) | W1-NCORE, W2-NDP, W2-NMEM, W3-NINT |
//!
//! Registration of work-package functions is static (`uop::k::family`, `decode::lower_insn`,
//! the pass calls in `trace::build`), so work packages never edit each other's files.
//!
//! `unsafe` is denied here; only `native/rt.rs` allows it.

#![deny(unsafe_code)]

pub mod backend;
pub mod bus;
pub mod cache;
pub mod config;
pub mod decode;
#[cfg(test)]
mod difftest;
pub mod exec;
pub mod exec_alu;
pub mod exec_flow;
pub mod exec_mem;
pub mod flagopt;
pub mod fuse;
pub mod hot;
pub mod idle;
pub mod loopwatch;
pub mod lower_dp;
pub mod lower_flow;
pub mod lower_mem;
pub mod lower_mul;
#[cfg(test)]
mod mutants;
#[cfg(feature = "sb-native")]
pub mod native;
pub mod pred;
pub mod run;
pub mod shadow;
pub mod stale;
pub mod stats;
#[cfg(test)]
mod testgen;
#[cfg(test)]
mod testkit;
pub mod trace;
pub mod uop;
pub mod validate;

pub use backend::{Backend, RunOut};
pub use bus::SbBus;
pub use config::SbConfig;
pub use decode::{decode, Insn};
pub use stats::SbStats;
pub use uop::{Exit, ExitKind, Sb, SbId, Uop, NO_SB};

/// Idle-skipping v2 hook: the loop-watch state between dispatch points (stage 2).
#[derive(Clone, Copy, Default, Debug)]
pub struct Watch {
    pub armed: bool,
    pub hit: bool,
    pub pc: u32,
}

/// All state of the tiers. It lives next to the `Machine` (tests, tools) and, after the seam, in
/// `Machine::sb`; the driver takes it out of the machine while it runs (`SbState::take_real`: a
/// placeholder stays behind and records every page that goes stale meanwhile, so none is lost),
/// so tier code can borrow the machine and its own state at once.
#[derive(Default)]
pub struct SbState {
    pub cfg: SbConfig,
    pub cache: cache::SbCache,
    pub hot: hot::HotTable,
    pub stats: SbStats,
    pub watch: Watch,
    pub backend: Option<Box<dyn backend::Backend + Send>>,
    /// True for the inert state `Machine::sb` holds while the driver has the real one taken out
    /// (`SbState::take_real`): it records stale pages in `pending` instead of acting on them.
    pub placeholder: bool,
    /// Pages that went stale while this placeholder stood in for the real state; the driver
    /// forwards them with `SbState::absorb_pending` (stale.rs). Deduplicated, never read otherwise.
    pub pending: Vec<u32>,
}

impl Clone for SbState {
    /// Same configuration, everything else empty: a cloned `Machine` starts cold but correct.
    fn clone(&self) -> Self {
        Self {
            cfg: self.cfg.clone(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;

    #[test]
    fn cloning_the_state_keeps_the_configuration_and_drops_everything_else() {
        let mut st = SbState {
            cfg: SbConfig::on(),
            ..SbState::default()
        };
        st.cache.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        st.hot.bump(0x1000);
        st.stats.built = 3;
        st.watch.armed = true;
        st.backend = Some(Box::new(backend::Tier2Backend::default()));
        let c = st.clone();
        assert!(c.cfg.enabled && c.cfg.hot == 0);
        assert_eq!(
            (c.cache.live_count(), c.stats.built, c.watch.armed),
            (0, 0, false)
        );
        assert!(c.backend.is_none());
        assert_eq!(c.cache.lookup(0x1000), None);
    }

    #[test]
    fn the_default_state_is_off_and_empty() {
        let st = SbState::default();
        assert!(!st.cfg.enabled && !st.cfg.idle_v2 && !st.cfg.native);
        assert_eq!(st.cache.live_count(), 0);
    }

    #[test]
    fn the_state_and_the_machine_stay_send() {
        fn is_send<T: Send>() {}
        is_send::<SbState>();
        is_send::<crate::machine::Machine>();
    }
}
