//! Write hooks, dirty-page revalidation and retranslation (design 7.4).
//!
//! OWNER: W2-DRIVE. `uop_note_write` and `uop_note_all` are called from `invalidate_sram_cache`
//! and `invalidate_code_cache` (`decode_cache.rs`); they touch only `Core::uop_run`, never the
//! boxed `UopHot` (it is taken out while a block runs), and return at once while the engine is
//! not armed, so the cost with the engine off is one byte load per invalidation.
//!
//! Skeleton: the hooks and the dirty bookkeeping are complete. Revalidation drops every block on
//! a dirty page (exact, never stale); the package replaces that with the `code_hash` compare so an
//! overlay that reloads identical code keeps its blocks (design 7.4 rule 2), and compacts the
//! tombstones.
use super::hot::UopHot;
use super::lower;
use super::run::{CODE_PAGES, RF_SMC};
use crate::core::Core;

/// Merges overlapping or touching ranges, ascending.
fn merge(mut v: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    v.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(v.len());
    for (lo, hi) in v {
        match out.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
            _ => out.push((lo, hi)),
        }
    }
    out
}

impl Core {
    /// A write of `count` short words at `first_short` happened: marks the code pages it touches
    /// as dirty and raises the self-modifying-code flag that store handlers test.
    #[inline]
    pub(in crate::core) fn uop_note_write(&mut self, first_short: u32, count: u32) {
        if !self.uop_run.active {
            return;
        }
        let mask = self.uop_run.code_pages.hit_mask(first_short, count);
        if mask != 0 {
            self.uop_run.dirty |= mask;
            self.uop_run.flags |= RF_SMC;
            self.uop_run.stats.smc += 1;
        }
    }

    /// The whole code cache was flushed (program change, ISA reload): every page is dirty.
    #[inline]
    pub(in crate::core) fn uop_note_all(&mut self) {
        if !self.uop_run.active {
            return;
        }
        self.uop_run.dirty = u64::MAX;
        self.uop_run.flags |= RF_SMC;
        self.uop_run.stats.smc += 1;
    }

    /// Bitmap words with a page written since the last revalidation (bit `k` is bitmap word `k`).
    pub fn uop_dirty(&self) -> u64 {
        self.uop_run.dirty
    }

    /// Whether `uop_maintain` has work: a page was written since the last revalidation, or
    /// revalidation dropped blocks whose ranges are not retranslated yet.
    pub fn uop_needs_maintenance(&self) -> bool {
        self.uop_run.dirty != 0 || self.uop.as_ref().is_some_and(|h| !h.dropped.is_empty())
    }

    /// Drops the blocks of every dirty page and clears the dirty state. Returns the blocks dropped.
    /// The dropped ranges are kept in `hot.dropped` for [`Core::uop_maintain`]; until then their
    /// pcs miss the pc map and run through `Core::step`.
    pub(in crate::core) fn uop_revalidate_dirty(&mut self, hot: &mut UopHot) -> usize {
        let dirty = std::mem::take(&mut self.uop_run.dirty);
        self.uop_run.flags &= !RF_SMC;
        let mut dropped = 0;
        for page in 0..CODE_PAGES {
            if dirty >> (page >> 6) & 1 == 0 {
                continue;
            }
            for id in std::mem::take(&mut hot.page_blocks[page]) {
                dropped += usize::from(hot.drop_block(id));
            }
        }
        dropped
    }

    /// Revalidates dirty pages and retranslates the dropped ranges; returns the blocks added.
    /// May allocate: it runs at a pass boundary, only after a program change or an overlay load
    /// (`maintain_calls` stays 0 in steady passes, gate G2).
    pub fn uop_maintain(&mut self) -> usize {
        if !self.uop_run.active {
            return 0;
        }
        let Some(mut hot) = self.uop.take() else {
            return 0;
        };
        self.uop_run.stats.maintain_calls += 1;
        self.uop_revalidate_dirty(&mut hot);
        let cfg = self.uop_run.cfg.clone();
        let mut added = 0;
        for (lo, hi) in merge(std::mem::take(&mut hot.dropped)) {
            if let Ok((blocks, uops)) = lower::translate_range(self, &cfg, lo, hi) {
                self.uop_run.stats.translations += blocks.len() as u64;
                added += hot.install(blocks, uops).0 as usize;
            }
        }
        self.uop = Some(hot);
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::config::UopConfig;

    #[test]
    fn merge_joins_overlapping_and_touching_ranges() {
        assert_eq!(
            merge(vec![(10, 20), (0, 4), (18, 30), (30, 31), (40, 41)]),
            [(0, 4), (10, 31), (40, 41)]
        );
        assert!(merge(Vec::new()).is_empty());
    }

    #[test]
    fn a_write_into_a_marked_page_sets_dirty_and_the_smc_flag_and_other_writes_do_not() {
        let mut c = core_with(&[0x0001; 4]);
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        c.uop_note_write(CODE + 0x4000, 2);
        assert_eq!((c.uop_dirty(), c.uop_run.flags), (0, 0), "an unmarked page");
        c.uop_note_write(CODE + 3, 1);
        assert_eq!(c.uop_dirty().count_ones(), 1);
        assert_eq!(c.uop_run.flags & RF_SMC, RF_SMC);
        c.uop_note_all();
        assert_eq!(c.uop_dirty(), u64::MAX);
        assert_eq!(c.uop_stats().smc, 2);
    }

    #[test]
    fn a_store_through_the_memory_path_reaches_the_hook_and_maintain_clears_it() {
        let mut c = core_with(&[0x0001; 4]);
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        // a data write of the normal word that holds the first two code parcels
        c.dm_write(CODE / 2, 0x1234_5678).unwrap();
        assert_ne!(c.uop_dirty(), 0, "dm_write reached invalidate_sram_cache");
        assert_eq!(c.uop_maintain(), 0, "the skeleton translates nothing");
        assert_eq!((c.uop_dirty(), c.uop_run.flags & RF_SMC), (0, 0));
        assert_eq!(c.uop_stats().maintain_calls, 1);
    }

    #[test]
    fn a_block_on_a_written_page_is_dropped_at_the_next_try_and_maintain_retranslates_its_range() {
        use crate::core::uop::lower::{builder::BlockBuilder, decode::decode_at};
        let mut c = core_with(&[0x0001; 4]);
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 4)])
            .unwrap();
        let mut b = BlockBuilder::new(CODE, false, false);
        let i = decode_at(&mut c, CODE);
        b.interp(&i);
        let (block, uops) = b.finish(Default::default());
        c.uop.as_mut().unwrap().install(vec![block], uops);
        assert!(!c.uop_needs_maintenance());
        c.dm_write(CODE / 2, 0x55).unwrap();
        assert!(c.uop_needs_maintenance(), "dirty");
        c.irq.due = u64::MAX;
        assert!(!c.uop_try(10), "the stale block must not run");
        assert_eq!(c.uop_dirty(), 0);
        assert!(c.uop_needs_maintenance(), "dropped, not yet retranslated");
        assert!(c.uop.as_ref().unwrap().pcmap(false).get(CODE).is_none());
        assert_eq!(c.uop_maintain(), 0, "the skeleton translates nothing");
        assert!(!c.uop_needs_maintenance());
    }

    #[test]
    fn maintain_on_a_disabled_engine_does_nothing() {
        let mut c = core_with(&[0x0001]);
        assert_eq!(c.uop_maintain(), 0);
        assert_eq!(c.uop_stats().maintain_calls, 0);
    }
}
