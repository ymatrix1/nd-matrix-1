//! Revalidation of the blocks on dirty pages (design 7.4 rule 2, P6, P14).
//!
//! OWNER: W2-DRIVE. A write marks code pages dirty (`uop_note_write`, one bit per 64 pages); the
//! next block entry calls [`Core::uop_revalidate_dirty`]. It walks only the blocks registered on
//! the pages of the dirty bitmap words (`page_blocks`), recomputes
//! `lower::decode::code_hash(core, block.pc, block.end_pc)` (the function the translator stored
//! the hash with) and
//!
//! * keeps a block whose hash still matches: an overlay that reloads identical code, or a data
//!   store that shares a page with code, costs a compare and no retranslation;
//! * drops the others: `UopHot::drop_block` tombstones the block, clears its pc-map entries,
//!   pushes its range to `hot.dropped` and bumps the epoch so cached links revalidate on use.
//!
//! The walk is in place: a kept block stays in its page list, a dropped one is removed with
//! `swap_remove`, and a tombstone met on the way (a block that spans two pages and was dropped
//! from the other one) is forgotten. Nothing allocates and nothing is freed (P14: the revalidation
//! runs on the audio path after a store into translated code). VERIFIED: `drop_block` itself by
//! `revalidation_and_drop_allocate_nothing` in this file.
use crate::core::uop::hot::UopHot;
use crate::core::uop::lower::decode::code_hash;
use crate::core::uop::run::{CODE_PAGES, CODE_PAGE_WORDS, RF_SMC};
use crate::core::Core;

/// Pages per bitmap word of `UopRun::dirty`: bit `k` of `dirty` is word `k` of the `CodePages`
/// bitmap, which holds 64 pages. VERIFIED: `CODE_PAGES / CODE_PAGE_WORDS` of `run.rs` (4096 / 64
/// = 64 = `u64::BITS`), checked by `a_dirty_bit_walks_exactly_the_64_pages_of_its_bitmap_word`
/// (the last page of word 0 is walked when bit 0 is set, the first page of word 1 only when bit
/// 1 is), max err 0; a value other than 64 fails it on one side or the other.
const PAGES_PER_WORD: usize = CODE_PAGES / CODE_PAGE_WORDS;

impl Core {
    /// Drops the blocks of every dirty page whose code changed, clears the dirty state and the
    /// self-modifying-code flag. Returns the blocks dropped. The dropped ranges are kept in
    /// `hot.dropped` for [`Core::uop_maintain`]; until then their pcs miss the pc map and run
    /// through `Core::step`.
    pub(in crate::core) fn uop_revalidate_dirty(&mut self, hot: &mut UopHot) -> usize {
        let dirty = std::mem::take(&mut self.uop_run.dirty);
        self.uop_run.flags &= !RF_SMC;
        let mut dropped = 0;
        for word in (0..CODE_PAGE_WORDS).filter(|w| dirty >> w & 1 != 0) {
            for page in word * PAGES_PER_WORD..(word + 1) * PAGES_PER_WORD {
                dropped += self.uop_revalidate_page(hot, page);
            }
        }
        dropped
    }

    /// The blocks of one page: forget tombstones, keep what matches, drop the rest.
    fn uop_revalidate_page(&self, hot: &mut UopHot, page: usize) -> usize {
        let (mut k, mut dropped) = (0, 0);
        while let Some(&id) = hot.page_blocks[page].get(k) {
            let b = &hot.blocks[id as usize];
            let stale = b.len == 0 || code_hash(self, b.pc, b.end_pc) != b.code_hash;
            if !stale {
                k += 1;
                continue;
            }
            dropped += usize::from(hot.drop_block(id));
            hot.page_blocks[page].swap_remove(k);
        }
        dropped
    }
}

#[cfg(test)]
mod tests {
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::drive::fixture::{arm_interp, nop_core};
    use crate::core::uop::run::CodePages;
    use crate::core::uop::verify::count_alloc::allocs_in;

    #[test]
    fn identical_code_keeps_the_blocks_and_changed_code_drops_exactly_those_blocks() {
        // two ranges, one block per mode each: ids 0 and 1 hold CODE.., ids 2 and 3 CODE + 0x100..
        let mut c = core_with(&[0x0001; 0x110]);
        arm_interp(&mut c, &[(CODE, CODE + 8), (CODE + 0x100, CODE + 0x108)]);
        let live = |c: &crate::core::Core| -> Vec<bool> {
            c.uop
                .as_ref()
                .unwrap()
                .blocks
                .iter()
                .map(|b| b.len != 0)
                .collect()
        };
        assert_eq!(live(&c), [true; 4]);
        // the same two parcels again: dirty, but every hash matches
        c.dm_write(CODE / 2, 0x0001_0001).unwrap();
        let mut hot = c.uop.take().unwrap();
        assert_eq!(c.uop_revalidate_dirty(&mut hot), 0);
        c.uop = Some(hot);
        assert_eq!((c.uop_dirty(), c.uop_run.flags), (0, 0));
        assert_eq!(live(&c), [true; 4], "an identical overlay keeps its blocks");
        assert!(!c.uop_needs_maintenance());
        // a changed parcel inside the second range drops its two blocks only; the first range
        // is on another page of the same bitmap word and is hashed again and kept
        c.dm_write((CODE + 0x102) / 2, 0x0001_0002).unwrap();
        let mut hot = c.uop.take().unwrap();
        assert_eq!(c.uop_revalidate_dirty(&mut hot), 2);
        let dropped = hot.dropped.clone();
        c.uop = Some(hot);
        assert_eq!(live(&c), [true, true, false, false]);
        assert_eq!(
            dropped,
            [
                (CODE + 0x100, CODE + 0x107 + 3),
                (CODE + 0x100, CODE + 0x107 + 3)
            ]
        );
        let hot = c.uop.as_ref().unwrap();
        assert_eq!(hot.pcmap(false).get(CODE + 0x101), None);
        assert_eq!(hot.pcmap(true).get(CODE + 0x101), None);
        assert!(
            hot.pcmap(false).get(CODE + 1).is_some(),
            "the other range is untouched"
        );
        assert_eq!(hot.epoch, 2, "one bump per dropped block");
        let page = CodePages::page_of(CODE + 0x100).unwrap();
        assert_eq!(
            hot.page_blocks[page],
            [] as [u32; 0],
            "dropped blocks leave their page list"
        );
    }

    #[test]
    fn a_block_that_spans_two_pages_is_forgotten_by_the_page_it_was_not_dropped_from() {
        // 0x20 NOPs from CODE + 0xF0: the block covers the pages of CODE and CODE + 0x100
        let mut c = core_with(&[0x0001; 0x110]);
        arm_interp(&mut c, &[(CODE + 0xF0, CODE + 0x110)]);
        let (p0, p1) = (
            CodePages::page_of(CODE).unwrap(),
            CodePages::page_of(CODE + 0x100).unwrap(),
        );
        {
            let hot = c.uop.as_ref().unwrap();
            assert_eq!(
                (hot.page_blocks[p0].len(), hot.page_blocks[p1].len()),
                (2, 2)
            );
        }
        c.dm_write((CODE + 0xF8) / 2, 0x0001_0002).unwrap();
        let mut hot = c.uop.take().unwrap();
        c.uop_revalidate_dirty(&mut hot);
        // both pages sit in the same bitmap word: both lists were walked and are empty
        assert!(hot.page_blocks[p0].is_empty() && hot.page_blocks[p1].is_empty());
        // a tombstone left on a page the walk did not reach is dropped the next time it is walked
        hot.page_blocks[p0].push(0);
        assert!(hot.blocks[0].len == 0);
        c.uop_run.dirty = u64::MAX;
        assert_eq!(
            c.uop_revalidate_dirty(&mut hot),
            0,
            "a tombstone is not a second drop"
        );
        assert!(hot.page_blocks[p0].is_empty());
    }

    #[test]
    fn a_dirty_bit_walks_exactly_the_64_pages_of_its_bitmap_word() {
        use crate::core::uop::run::CODE_BASE;
        use crate::core::Core;
        // one range on the last page of bitmap word 0 (page 63) and one on the first page of
        // word 1 (page 64); both are changed, but only the dirty bit says which one is walked
        let (a, b) = (CODE_BASE + (63 << 8), CODE_BASE + (64 << 8));
        assert_eq!(
            (CodePages::page_of(a), CodePages::page_of(b)),
            (Some(63), Some(64))
        );
        let mut c = Core::new(&[]);
        for pc in [a, b] {
            for k in 0..8 {
                c.dm_write(pc + k, 0x0001).unwrap();
            }
        }
        arm_interp(&mut c, &[(a, a + 8), (b, b + 8)]);
        c.dm_write(a, 0x0002).unwrap();
        c.dm_write(b, 0x0002).unwrap();
        assert_eq!(c.uop_dirty(), 0b11, "one bit per bitmap word");
        let live = |c: &Core| -> Vec<bool> {
            let hot = c.uop.as_ref().unwrap();
            hot.blocks.iter().map(|k| k.len != 0).collect()
        };
        c.uop_run.dirty = 1; // word 0 only
        let mut hot = c.uop.take().unwrap();
        assert_eq!(c.uop_revalidate_dirty(&mut hot), 2, "page 63: both modes");
        c.uop = Some(hot);
        assert_eq!(
            live(&c),
            [false, false, true, true],
            "page 64 was not walked"
        );
        c.uop_run.dirty = 1 << 1; // word 1
        let mut hot = c.uop.take().unwrap();
        assert_eq!(c.uop_revalidate_dirty(&mut hot), 2, "page 64: both modes");
        c.uop = Some(hot);
        assert_eq!(live(&c), [false; 4]);
        assert_eq!(super::PAGES_PER_WORD, u64::BITS as usize);
    }

    #[test]
    fn revalidation_and_drop_allocate_nothing() {
        // P14: the revalidation runs on the audio path after a store into translated code
        let mut c = nop_core(40);
        arm_interp(&mut c, &[(CODE, CODE + 40)]);
        c.dm_write((CODE + 8) / 2, 0x0001_0002).unwrap();
        let mut hot = c.uop.take().unwrap();
        let (dropped, allocs) = allocs_in(|| c.uop_revalidate_dirty(&mut hot));
        assert_eq!((dropped, allocs), (2, 0));
        // identical code: the compare allocates nothing either
        let mut again = nop_core(40);
        arm_interp(&mut again, &[(CODE, CODE + 40)]);
        again.dm_write((CODE + 8) / 2, 0x0001_0001).unwrap();
        let mut hot2 = again.uop.take().unwrap();
        let (dropped, allocs) = allocs_in(|| again.uop_revalidate_dirty(&mut hot2));
        assert_eq!((dropped, allocs), (0, 0));
        // and `drop_block` alone, on a table with the capacity `install` reserved
        let (_, allocs) = allocs_in(|| hot2.drop_block(0));
        assert_eq!(allocs, 0);
        c.uop = Some(hot);
    }
}
