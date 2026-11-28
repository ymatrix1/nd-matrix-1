//! The uop engine on `Core`: arming, the block-entry hook and the run loop.
//!
//! OWNER: W2-DRIVE. `enable_uop` translates the given short-word ranges and arms the write hooks;
//! `uop_try` is the analogue of `aot_try`: it runs a block when the pc is at a uop boundary, the
//! entry guard holds and the exit protocol (design 4.5) can account the result, and returns
//! false otherwise so the caller runs `Core::step`. Nothing here is called unless a caller
//! enables the engine; with it off, `uop_try` is one byte load and a branch.
//!
//! Skeleton: translation yields no blocks, so `uop_try` finds no block and returns false and
//! `run_uop` equals `Core::run`. The single-block path below is complete for what it does (guard,
//! room, exit and counters); chaining through `Link`s, fault replay and the `Smc` re-dispatch are
//! W2-DRIVE.
use super::config::{UopBuildReport, UopConfig, UopError, UopStats};
use super::exec::{self, event, exit};
use super::hot::UopHot;
use super::lower::{self, guard};
use super::run::{UopRun, CODE_BASE, CODE_SPAN};
use super::types::{Exit, NONE};
use crate::core::{Core, Fault};

impl Core {
    /// Translates the short-word ranges `[lo, hi)` and arms the engine. Never called by default.
    /// On an error the engine is left as it was.
    pub fn enable_uop(
        &mut self,
        cfg: UopConfig,
        ranges: &[(u32, u32)],
    ) -> Result<UopBuildReport, UopError> {
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
        let mut hot = UopHot::new();
        let mut report = UopBuildReport::default();
        for &(lo, hi) in ranges {
            match lower::translate_range(self, &cfg, lo, hi) {
                Ok((blocks, uops)) => {
                    report.insns += uops.iter().map(|u| u32::from(u.steps)).sum::<u32>();
                    report.interp_insns += uops
                        .iter()
                        .filter(|u| u.h == super::types::UopKind::INTERP.0)
                        .count() as u32;
                    let (nb, nu) = hot.install(blocks, uops);
                    (report.blocks, report.uops) = (report.blocks + nb, report.uops + nu);
                }
                Err(e) => {
                    self.disable_uop();
                    return Err(e);
                }
            }
            self.uop_run.code_pages.set_range(lo, hi);
        }
        report.pages = hot.page_blocks.iter().filter(|v| !v.is_empty()).count() as u32;
        self.uop_run.stats.translations = u64::from(report.blocks);
        self.uop_run.cfg = cfg;
        self.uop_run.stop_pc = stop_pc;
        self.uop_run.active = true;
        self.uop = Some(Box::new(hot));
        Ok(report)
    }

    /// Turns the engine off and drops every block and counter.
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

    /// pc at which block chaining must return to the caller (the pass idle pc); `None` clears it.
    pub fn uop_set_stop_pc(&mut self, pc: Option<u32>) {
        self.uop_run.stop_pc = pc.unwrap_or(NONE);
    }

    /// The fault an `Interp` uop raised, if any (it also stopped the block).
    pub fn uop_take_fault(&mut self) -> Option<Fault> {
        self.uop_run.fault.take()
    }

    /// Runs one block when `limit` leaves room and the guard holds; returns whether any step ran.
    /// A fault is left for [`Core::uop_take_fault`]. Same contract as `aot_try`.
    #[inline]
    pub fn uop_try(&mut self, limit: u64) -> bool {
        self.uop_run.active && self.uop_enter(limit)
    }

    #[inline(never)]
    fn uop_enter(&mut self, limit: u64) -> bool {
        let Some(mut hot) = self.uop.take() else {
            return false;
        };
        let ran = self.uop_run_block(&mut hot, limit);
        self.uop = Some(hot);
        // the caller runs exactly one `Core::step` after a false return
        self.uop_run.stats.steps_core_step += u64::from(!ran);
        ran
    }

    /// Marks the start of a pass: counts it and sets the pc at which chaining returns (the idle
    /// pc of the pass). No-op while the engine is off.
    pub fn uop_begin_pass(&mut self, idle_pc: u32) {
        if self.uop_run.active {
            self.uop_run.stats.passes += 1;
            self.uop_run.stop_pc = idle_pc;
        }
    }

    /// The block at the pc, if the guard lets it run: runs it and accounts the exit.
    fn uop_run_block(&mut self, hot: &mut UopHot, limit: u64) -> bool {
        if self.uop_run.dirty != 0 {
            self.uop_revalidate_dirty(hot);
        }
        let Some((id, from)) = hot.pcmap(self.simd()).get(self.pc) else {
            return false;
        };
        let block = &hot.blocks[id as usize];
        let room = event::room(self, limit);
        if let Some(row) = guard::check(self, block, room) {
            self.uop_run.stats.guard_fail[row as usize] += 1;
            return false;
        }
        let whole = room >= block.steps;
        let native = block.native != NONE;
        // SAFETY: `hot` is the table the block was built for, `id` and `from` come from its pc
        // map, and no other reference to the core is alive: `hot` was taken out of `self.uop`.
        let (exit, at, done) = unsafe {
            if native {
                self.uop_native(hot, id, room)
            } else if whole {
                exec::run_block(self, hot, id, from, room)
            } else {
                exec::run_block_checked(self, hot, id, from, room)
            }
        };
        let st = &mut self.uop_run.stats;
        st.blocks_run += 1;
        st.uops += u64::from(at.saturating_sub(from));
        st.steps_uop += u64::from(done);
        exit::count_exit(st, exit);
        done > 0 || (exit == Exit::Fault && self.uop_run.fault.is_some())
    }

    /// Runs block `id` through the native tier.
    ///
    /// # Safety
    /// As `exec::run_block`.
    #[cfg(feature = "uop-native")]
    unsafe fn uop_native(&mut self, hot: &UopHot, id: u32, room: u32) -> (Exit, u32, u32) {
        super::native::enter(self, hot, id, room)
    }

    /// Without the native tier no block carries a native entry; the interpreter runs it.
    ///
    /// # Safety
    /// None needed; `unsafe` only to match the native variant.
    #[cfg(not(feature = "uop-native"))]
    unsafe fn uop_native(&mut self, _hot: &UopHot, _id: u32, _room: u32) -> (Exit, u32, u32) {
        (Exit::Slow, 0, 0)
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
    use crate::core::uop::lower::builder::BlockBuilder;
    use crate::core::uop::lower::decode::decode_at;
    use crate::core::uop::run::CodePages;
    use crate::core::uop::verify::compare::exact_diff;

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
    }

    #[test]
    fn enable_arms_the_hooks_and_disable_restores_the_inert_state() {
        let mut c = core_with(&[0x0001; 4]);
        let r = c
            .enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        assert_eq!(
            r,
            UopBuildReport::default(),
            "the skeleton translates nothing"
        );
        assert!(c.uop_enabled() && c.uop_run.code_pages.test(CodePages::page_of(CODE).unwrap()));
        c.uop_set_stop_pc(Some(CODE + 3));
        assert_eq!(c.uop_run.stop_pc, CODE + 3);
        c.uop_set_stop_pc(None);
        assert_eq!(c.uop_run.stop_pc, NONE);
        c.disable_uop();
        assert!(!c.uop_enabled() && c.uop_run.code_pages.is_empty() && c.uop.is_none());
    }

    #[test]
    fn a_pc_without_a_block_is_left_to_core_step_and_counts_nothing() {
        let mut c = core_with(&[0x0001; 4]);
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        assert!(!c.uop_try(100));
        let one_core_step = UopStats {
            steps_core_step: 1,
            ..UopStats::default()
        };
        assert_eq!(
            c.uop_stats(),
            one_core_step,
            "the caller now runs one Core::step"
        );
        assert_eq!(c.run_uop(3), None);
        assert_eq!(
            (c.steps, c.pc, c.uop_stats().steps_core_step),
            (3, CODE + 3, 4)
        );
    }

    #[test]
    fn a_block_the_skeleton_executor_declines_changes_nothing_and_counts_one_slow_exit() {
        let mut c = core_with(&[0x0001; 4]);
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        let mut b = BlockBuilder::new(CODE, false, false);
        let mut pc = CODE;
        for _ in 0..3 {
            let i = decode_at(&mut c, pc);
            b.interp(&i);
            pc = i.next_pc;
        }
        let (block, uops) = b.finish(Default::default());
        c.uop.as_mut().unwrap().install(vec![block], uops);
        let mut want = core_with(&[0x0001; 4]);
        (c.irq.due, want.irq.due) = (u64::MAX, u64::MAX); // no event pending: the room is the limit
        assert!(
            !c.uop_try(100),
            "run_block is a stub: Slow with no step done"
        );
        assert_eq!(exact_diff(&c, &want), None);
        let s = c.uop_stats();
        assert_eq!(
            (s.blocks_run, s.exits[Exit::Slow as usize], s.steps_uop),
            (1, 1, 0)
        );
        // the run loop still reaches the interpreter result
        assert_eq!(c.run_uop(4), None);
        assert_eq!(want.run(4), None);
        assert_eq!(exact_diff(&c, &want), None);
    }

    #[test]
    fn the_entry_guard_counts_its_row() {
        let mut c = core_with(&[0x0001; 4]);
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        let mut b = BlockBuilder::new(CODE, false, false);
        let i = decode_at(&mut c, CODE);
        b.interp(&i);
        let (mut block, uops) = b.finish(Default::default());
        block.poisoned = true;
        c.uop.as_mut().unwrap().install(vec![block], uops);
        c.irq.due = u64::MAX;
        assert!(!c.uop_try(100));
        assert_eq!(
            c.uop_stats().guard_fail[super::super::types::GuardFail::Valid as usize],
            1
        );
    }

    #[test]
    fn a_block_translated_for_the_other_simd_mode_is_not_in_the_map_of_this_one() {
        let mut c = core_with(&[0x0001; 4]);
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        let mut b = BlockBuilder::new(CODE, true, false); // translated for PEy on, core has it off
        let i = decode_at(&mut c, CODE);
        b.interp(&i);
        let (block, uops) = b.finish(Default::default());
        c.uop.as_mut().unwrap().install(vec![block], uops);
        c.irq.due = u64::MAX;
        assert!(!c.uop_try(100));
        let s = c.uop_stats();
        assert_eq!(
            (s.blocks_run, s.guard_fail),
            (0, [0; 9]),
            "no entry, no guard row"
        );
        c.mode1 |= 1 << 21;
        assert!(!c.uop_try(100), "the skeleton executor declines");
        assert_eq!(
            c.uop_stats().blocks_run,
            1,
            "the PEy-on map found the block"
        );
    }
}
