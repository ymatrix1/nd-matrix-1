//! Two-tier exact uop engine for the SHARC core (D260, `docs/specs/sharc-uop.md`).
//!
//! Tier 1 translates code ranges into blocks of 16-byte uops that a handler table runs with a
//! static step budget and exact bail-out to `Core::step`; Tier 2 (feature `uop-native`) emits
//! native code for the same blocks with Cranelift. `Core::step` stays the only oracle: every
//! block materializes the interpreter state at a uop boundary on every exit, and anything the
//! lowering does not model is an `Interp` uop that runs the interpreter body verbatim.
//!
//! Nothing here runs unless a caller arms it with `Core::enable_uop`. This module only declares
//! the tree; the one wiring test below pins that the disabled engine changes nothing.
pub mod config;
pub mod drive;
pub mod exec;
pub mod hot;
pub mod invalidate;
pub mod kernels;
pub mod lower;
#[cfg(feature = "uop-native")]
pub mod native;
pub mod run;
pub mod service;
pub mod types;
pub mod verify;

pub use config::{UopBuildReport, UopConfig, UopError, UopStats};
pub(crate) use hot::UopHot;
pub(crate) use run::UopRun;
pub use types::{Block, Exit, Footprint, GuardClass, Handler, Link, Uop, UopCtx, UopKind, NONE};
pub use verify::compare::ExactDiff;
pub use verify::shadow::{shadow_run_with, Shadow};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};

    #[test]
    fn hook_is_noop_when_disabled() {
        let mut c = core_with(&[0x0001; 4]);
        c.uop_note_write(CODE, 4);
        c.uop_note_all();
        c.dm_write(CODE / 2, 0x1234_5678).unwrap(); // reaches invalidate_sram_cache
        c.dm_write(CODE + 1, 7).unwrap(); // the short-word alias window
        assert_eq!((c.uop_dirty(), c.uop_run.flags), (0, 0));
        assert_eq!(c.uop_stats(), UopStats::default());
        assert!(!c.uop_enabled() && !c.uop_try(1_000) && c.uop_take_fault().is_none());
        assert_eq!(c.uop_maintain(), 0);
        assert!(c.uop.is_none() && c.uop_run.code_pages.is_empty());
    }

    #[test]
    fn a_core_that_never_arms_the_engine_runs_exactly_like_the_interpreter() {
        let (mut a, mut b) = (core_with(&[0x0001; 6]), core_with(&[0x0001; 6]));
        assert_eq!((a.run_uop(5), b.run(5)), (None, None));
        assert_eq!(verify::compare::exact_diff(&a, &b), None);
        assert_eq!((a.steps, a.pc), (5, CODE + 5));
    }

    #[test]
    fn the_public_surface_is_the_documented_one() {
        let cfg = UopConfig::default();
        assert_eq!(
            (cfg.min_block_steps, cfg.max_block_steps, cfg.native),
            (1, 48, false)
        );
        assert_eq!(usize::from(UopKind::MAX), 0x500);
        assert_eq!(std::mem::size_of::<Uop>(), 16);
        assert_eq!(Exit::from_u32(Exit::Smc as u32), Some(Exit::Smc));
        assert_eq!(NONE, u32::MAX);
    }
}
