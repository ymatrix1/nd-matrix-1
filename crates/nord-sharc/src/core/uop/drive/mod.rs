//! The uop engine on `Core`: arming, the lifecycle and the run loop.
//!
//! OWNER: W2-DRIVE. `enable_uop` translates the given short-word ranges and arms the write hooks;
//! `chain.rs` holds `uop_try`, the analogue of `aot_try`; `pass.rs` the stop pc and the start of a
//! pass. Nothing here is called unless a caller enables the engine; with it off, `uop_try` is one
//! byte load and a branch.
//!
//! `enable_uop` keeps `uop_run.stop_pc`: the translator reads it to make the pass idle pc a block
//! start (design P7), and `disable_uop` (which resets the run state) used to run in between, so a
//! block could run past the pc the pass ends on. The fix is the two lines in `uop_arm_with`; test
//! `a_stop_pc_set_before_enable_makes_it_a_block_start`.
mod chain;
#[cfg(test)]
mod equiv;
#[cfg(test)]
mod faults;
#[cfg(test)]
pub(crate) mod fixture;
mod pass;

use super::config::{UopBuildReport, UopConfig, UopError, UopStats};
use super::hot::UopHot;
use super::lower::builder::BlockBuilder;
use super::lower::{self, translate, LowerCtx};
use super::run::{UopRun, CODE_BASE, CODE_SPAN};
use super::types::UopKind;
use crate::core::decode_cache::CachedInsn;
use crate::core::{Core, Fault};

impl Core {
    /// Translates the short-word ranges `[lo, hi)` and arms the engine. Never called by default.
    /// A range that is empty or outside the code window is an error and leaves the engine as it
    /// was. The stop pc ([`Core::uop_set_stop_pc`]) survives and is a block start.
    pub fn enable_uop(
        &mut self,
        cfg: UopConfig,
        ranges: &[(u32, u32)],
    ) -> Result<UopBuildReport, UopError> {
        self.uop_arm_with(cfg, ranges, lower::lower_insn)
    }

    /// [`Core::enable_uop`] with another lowering. The tests of the driver arm the engine with an
    /// all-`Interp` lowering (exact by construction, and still the same when the fast families
    /// land), so what they check is the engine and not a family.
    pub(in crate::core) fn uop_arm_with<L>(
        &mut self,
        cfg: UopConfig,
        ranges: &[(u32, u32)],
        lower: L,
    ) -> Result<UopBuildReport, UopError>
    where
        L: Fn(&LowerCtx<'_>, &CachedInsn, &mut BlockBuilder),
    {
        if cfg.native && !cfg!(feature = "uop-native") {
            return Err(UopError::NativeUnavailable);
        }
        let window = CODE_BASE..CODE_BASE + CODE_SPAN;
        if let Some(&(lo, hi)) = ranges
            .iter()
            .find(|(lo, hi)| lo >= hi || !window.contains(lo) || *hi > window.end)
        {
            return Err(UopError::OutOfRange { lo, hi });
        }
        let stop_pc = self.uop_run.stop_pc;
        self.disable_uop();
        self.uop_run.stop_pc = stop_pc;
        let mut hot = UopHot::new();
        let mut report = UopBuildReport::default();
        for &(lo, hi) in ranges {
            let (blocks, uops) = translate::translate_with(self, &cfg, lo, hi, &lower);
            report.insns += uops.iter().map(|u| u32::from(u.steps)).sum::<u32>();
            report.interp_insns += uops.iter().filter(|u| u.h == UopKind::INTERP.0).count() as u32;
            let (nb, nu) = hot.install(blocks, uops);
            (report.blocks, report.uops) = (report.blocks + nb, report.uops + nu);
            self.uop_run.code_pages.set_range(lo, hi);
        }
        report.pages = hot.page_blocks.iter().filter(|v| !v.is_empty()).count() as u32;
        self.uop_run.stats.translations = u64::from(report.blocks);
        self.uop_run.cfg = cfg;
        self.uop_run.active = true;
        self.uop = Some(Box::new(hot));
        Ok(report)
    }

    /// Turns the engine off and drops every block, counter and the stop pc.
    pub fn disable_uop(&mut self) {
        self.uop = None;
        self.uop_run = UopRun::new();
    }

    /// Whether the engine is armed.
    pub fn uop_enabled(&self) -> bool {
        self.uop_run.active
    }

    /// Counters since `enable_uop` or the last [`Core::uop_reset_stats`].
    pub fn uop_stats(&self) -> UopStats {
        self.uop_run.stats
    }

    /// Zeroes the counters.
    pub fn uop_reset_stats(&mut self) {
        self.uop_run.stats = UopStats::default();
    }

    /// The fault an `Interp` uop raised (or a replayed uop, `chain.rs`), if any. It also stopped
    /// the block, and the core is in the state of the faulting instruction like after a failed
    /// `Core::step`.
    pub fn uop_take_fault(&mut self) -> Option<Fault> {
        self.uop_run.fault.take()
    }

    /// [`Core::run`] with the block hook: identical results, but a translated block runs as one
    /// call whenever the pc reaches a uop boundary with room for it.
    pub fn run_uop(&mut self, max_steps: u64) -> Option<Fault> {
        let limit = self.steps + max_steps;
        while self.steps < limit {
            if self.uop_try(limit) {
                if let Some(f) = self.uop_take_fault() {
                    return Some(f);
                }
                continue;
            }
            if let Err(f) = self.step() {
                return Some(f);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::drive::fixture::{arm_interp, nop_core};
    use crate::core::uop::run::CodePages;
    use crate::core::uop::types::NONE;

    #[test]
    fn enable_validates_ranges_and_the_native_request_before_changing_anything() {
        let mut c = core_with(&[0x0001; 4]);
        for (lo, hi) in [
            (CODE + 2, CODE + 2),
            (CODE + 3, CODE),
            (0, 10),
            (CODE, 0x30_0000),
        ] {
            assert_eq!(
                c.enable_uop(UopConfig::default(), &[(lo, hi)]),
                Err(UopError::OutOfRange { lo, hi })
            );
        }
        assert!(!c.uop_enabled());
        let native = UopConfig {
            native: true,
            ..UopConfig::default()
        };
        let r = c.enable_uop(native, &[(CODE, CODE + 4)]);
        assert_eq!(r.is_err(), !cfg!(feature = "uop-native"));
        c.disable_uop();
        // an armed engine survives a refused re-arming
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        assert!(c.enable_uop(UopConfig::default(), &[(0, 10)]).is_err());
        assert!(c.uop_enabled() && c.uop.is_some());
    }

    #[test]
    fn enable_arms_the_hooks_and_disable_restores_the_inert_state() {
        let mut c = core_with(&[0x0001; 4]);
        let r = c
            .enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        // four NOPs make one block per PEy mode (design P4), each of four steps
        assert_eq!((r.blocks, r.insns, r.pages), (2, 8, 1));
        assert!(r.uops >= 2);
        assert_eq!(c.uop_stats().translations, 2);
        assert!(c.uop_enabled() && c.uop_run.code_pages.test(CodePages::page_of(CODE).unwrap()));
        c.uop_set_stop_pc(Some(CODE + 3));
        assert_eq!(c.uop_run.stop_pc, CODE + 3);
        c.uop_set_stop_pc(None);
        assert_eq!(c.uop_run.stop_pc, NONE);
        c.disable_uop();
        assert!(!c.uop_enabled() && c.uop_run.code_pages.is_empty() && c.uop.is_none());
        assert_eq!(c.uop_run.stop_pc, NONE, "disable resets the stop pc too");
    }

    #[test]
    fn re_enabling_replaces_the_blocks_and_the_counters() {
        let mut c = core_with(&[0x0001; 8]);
        let first = c
            .enable_uop(UopConfig::default(), &[(CODE, CODE + 8)])
            .unwrap();
        c.irq.due = u64::MAX;
        assert!(c.uop_try(100));
        assert_ne!(c.uop_stats().blocks_run, 0);
        let second = c
            .enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        assert!(second.uops < first.uops, "only the new range is translated");
        assert_eq!(c.uop_stats().blocks_run, 0, "the counters start again");
        c.uop_reset_stats();
        assert_eq!(c.uop_stats(), UopStats::default());
    }

    #[test]
    fn a_stop_pc_set_before_enable_makes_it_a_block_start() {
        // inherited finding (a): `enable_uop` saved the stop pc, `disable_uop` reset it and the
        // translator ran with none, so a block spanned the idle pc
        let mut c = core_with(&[0x0001; 12]);
        c.uop_set_stop_pc(Some(CODE + 5));
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 12)])
            .unwrap();
        assert_eq!(c.uop_run.stop_pc, CODE + 5, "kept across the re-arming");
        let hot = c.uop.as_ref().unwrap();
        for simd in [false, true] {
            let (id, idx) = hot
                .pcmap(simd)
                .get(CODE + 5)
                .expect("the stop pc is mapped");
            assert_eq!(
                (idx, hot.blocks[id as usize].pc),
                (0, CODE + 5),
                "a block starts at the stop pc (mode {simd})"
            );
            let (before, _) = hot.pcmap(simd).get(CODE + 4).expect("the NOP before it");
            assert_eq!(
                hot.blocks[before as usize].end_pc,
                CODE + 5,
                "the block in front of it ends there (mode {simd})"
            );
        }
        // and without a stop pc the twelve NOPs are one block per mode
        let mut free = core_with(&[0x0001; 12]);
        let r = free
            .enable_uop(UopConfig::default(), &[(CODE, CODE + 12)])
            .unwrap();
        assert_eq!(r.blocks, 2);
    }

    #[test]
    fn run_uop_equals_run_for_a_core_with_blocks_and_for_one_without() {
        let (mut want, mut plain, mut armed) = (nop_core(20), nop_core(20), nop_core(20));
        arm_interp(&mut armed, &[(CODE, CODE + 20)]);
        assert_eq!(
            (want.run(17), plain.run_uop(17), armed.run_uop(17)),
            (None, None, None)
        );
        assert_eq!(want.state_mismatch(&plain), None);
        assert_eq!(want.state_mismatch(&armed), None);
        let s = armed.uop_stats();
        assert_eq!(
            (s.steps_uop, s.steps_core_step),
            (s.steps_interp_uop, 0),
            "{s:?}"
        );
        assert_eq!(s.steps_uop, 17);
    }
}
