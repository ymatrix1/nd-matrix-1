//! Write hooks, dirty-page revalidation, retranslation and the asynchronous install path
//! (design 7.4).
//!
//! OWNER: W2-DRIVE. `uop_note_write` and `uop_note_all` are called from `invalidate_sram_cache`
//! and `invalidate_code_cache` (`decode_cache.rs`); they touch only `Core::uop_run`, never the
//! boxed `UopHot` (it is taken out while a block runs), and return at once while the engine is
//! not armed, so the cost with the engine off is one byte load per invalidation.
//!
//! The rest of the work happens at the next block entry or pass boundary:
//!
//! | File | What |
//! |---|---|
//! | `revalidate.rs` | `uop_revalidate_dirty`: walk the blocks of the dirty pages, keep those whose `code_hash` still matches (P6), drop the others (P14: no allocation) |
//! | `maintain.rs` | `uop_maintain`: revalidate, retranslate the dropped ranges, compact the tombstones |
//! | `install.rs` | `uop_maintain_async` and `uop_install_done`: the same through the translation service, hash-checked against the live core before a worker-built block is installed |
//! | `smc.rs` (tests) | the self-modifying-code rows: own block, other block, page of another block, alias write |
mod install;
mod maintain;
mod revalidate;
#[cfg(test)]
mod smc;

use super::run::RF_SMC;
use crate::core::Core;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::drive::fixture::{arm_interp, nop_core};

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
        let blocks = c.uop.as_ref().unwrap().blocks.len();
        assert_eq!(blocks, 2, "one block per PEy mode");
        // a data write of the normal word that holds the first two code parcels, with the same
        // two parcels it already holds
        c.dm_write(CODE / 2, 0x0001_0001).unwrap();
        assert_ne!(c.uop_dirty(), 0, "dm_write reached invalidate_sram_cache");
        assert!(c.uop_needs_maintenance());
        // the code did not change: the blocks stay, nothing is retranslated
        assert_eq!(c.uop_maintain(), 0, "an identical overlay costs a compare");
        assert_eq!((c.uop_dirty(), c.uop_run.flags & RF_SMC), (0, 0));
        assert_eq!(c.uop_stats().maintain_calls, 1);
        assert!(c.uop.as_ref().unwrap().blocks.iter().all(|b| b.len != 0));
        assert!(!c.uop_needs_maintenance());
    }

    #[test]
    fn the_code_cache_flush_marks_everything_dirty_and_a_disabled_engine_ignores_both_hooks() {
        let mut c = nop_core(8);
        c.invalidate_code_cache();
        assert_eq!((c.uop_dirty(), c.uop_stats().smc), (0, 0), "not armed");
        arm_interp(&mut c, &[(CODE, CODE + 8)]);
        c.invalidate_code_cache();
        assert_eq!(
            c.uop_dirty(),
            u64::MAX,
            "invalidate_code_cache reached uop_note_all"
        );
        c.disable_uop();
        assert_eq!((c.uop_dirty(), c.uop_run.flags), (0, 0));
    }

    #[test]
    fn maintain_on_a_disabled_engine_does_nothing() {
        let mut c = core_with(&[0x0001]);
        assert_eq!(c.uop_maintain(), 0);
        assert_eq!(c.uop_stats().maintain_calls, 0);
        assert!(!c.uop_needs_maintenance());
    }
}
