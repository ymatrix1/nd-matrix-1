//! OWNER: W1-CACHE. Dispatch counts that decide where a trace is built.
//!
//! Spec (design note 3.3, WP W1-CACHE): per page `Box<[u16; 1024]>` of dispatch counts plus a
//! per-page failure counter. `bump` is called at dispatch points only (tier-1 slice boundaries
//! and trace exits), so a loop of period `P` is sampled at the pcs that are multiples of `slice`
//! modulo `P`. A page is allocated by the first `bump` or `mark_failed` on it; every call after
//! that allocates nothing. The failure bits sit next to the counts so `failed` costs one load
//! when the page has none.

use super::cache::pagetable::PageTable;

/// Instruction words per 4 KiB page.
/// VERIFIED: `mem::PAGE_SIZE / 4`, checked by `a_page_has_one_count_per_word`.
const WORDS: usize = 1 << 10;

/// Counts and failure marks of one page.
struct PageHot {
    counts: Box<[u16; WORDS]>,
    /// One bit per word: no trace can be built there.
    failed: [u64; WORDS / 64],
    /// Number of set bits of `failed`.
    n_failed: u16,
}

impl PageHot {
    fn new() -> Self {
        Self {
            counts: Box::new([0; WORDS]),
            failed: [0; WORDS / 64],
            n_failed: 0,
        }
    }
}

/// Hotness of code addresses.
#[derive(Default)]
pub struct HotTable {
    pages: PageTable<PageHot>,
}

#[inline(always)]
fn word_index(pc: u32) -> usize {
    (pc >> 2) as usize & (WORDS - 1)
}

impl HotTable {
    /// Count one dispatch of `pc` (saturating) and return the new count.
    pub fn bump(&mut self, pc: u32) -> u16 {
        let c = &mut self.pages.get_or_insert_with(pc >> 12, PageHot::new).counts[word_index(pc)];
        *c = c.saturating_add(1);
        *c
    }

    /// Forget the count of `pc` (after a build, or when its trace dies).
    pub fn reset(&mut self, pc: u32) {
        if let Some(p) = self.pages.get_mut(pc >> 12) {
            p.counts[word_index(pc)] = 0;
        }
    }

    /// No trace can be built at `pc`: stop counting it (until the code changes).
    pub fn mark_failed(&mut self, pc: u32) {
        let p = self.pages.get_or_insert_with(pc >> 12, PageHot::new);
        let w = word_index(pc);
        p.counts[w] = 0;
        let (word, bit) = (w / 64, 1u64 << (w % 64));
        if p.failed[word] & bit == 0 {
            p.failed[word] |= bit;
            p.n_failed += 1;
        }
    }

    pub fn failed(&self, pc: u32) -> bool {
        self.pages.get(pc >> 12).is_some_and(|p| {
            let w = word_index(pc);
            p.n_failed != 0 && p.failed[w / 64] >> (w % 64) & 1 != 0
        })
    }

    /// The code of `page` (a page number) may have changed: every pc of it may be tried again.
    pub fn forget_failures(&mut self, page: u32) {
        if let Some(p) = self.pages.get_mut(page) {
            if p.n_failed != 0 {
                p.failed = [0; WORDS / 64];
                p.n_failed = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl HotTable {
        fn count(&self, pc: u32) -> u16 {
            self.pages
                .get(pc >> 12)
                .map_or(0, |p| p.counts[word_index(pc)])
        }
    }

    #[test]
    fn a_page_has_one_count_per_word() {
        assert_eq!(WORDS * 4, crate::mem::PAGE_SIZE);
        assert_eq!(WORDS % 64, 0);
    }

    #[test]
    fn bump_counts_per_pc_and_reset_forgets() {
        let mut h = HotTable::default();
        assert_eq!((h.bump(0x1000), h.bump(0x1000), h.bump(0x2000)), (1, 2, 1));
        assert_eq!((h.bump(0x1004), h.bump(0x1FFC), h.bump(0x2000)), (1, 1, 2));
        h.reset(0x1000);
        assert_eq!(
            (h.count(0x1000), h.count(0x1004), h.count(0x2000)),
            (0, 1, 2)
        );
        assert_eq!(h.bump(0x1000), 1);
        h.reset(0x9000); // a page never seen
        assert_eq!(h.count(0x9000), 0);
    }

    #[test]
    fn the_count_saturates_instead_of_wrapping() {
        let mut h = HotTable::default();
        let last = (0..70_000u32).map(|_| h.bump(0x1000)).last();
        assert_eq!(last, Some(u16::MAX));
        assert_eq!(h.bump(0x1000), u16::MAX);
    }

    #[test]
    fn a_failed_pc_is_remembered_and_its_count_dropped() {
        let mut h = HotTable::default();
        h.bump(0x1000);
        h.mark_failed(0x1000);
        assert!(h.failed(0x1000) && !h.failed(0x1004) && !h.failed(0x2000));
        assert_eq!(h.count(0x1000), 0);
        assert_eq!(h.bump(0x1000), 1, "the count restarted");
        assert!(h.failed(0x1000), "counting does not clear the mark");
    }

    #[test]
    fn marking_twice_counts_once_and_every_word_has_its_own_bit() {
        let mut h = HotTable::default();
        for w in (0..1024u32).step_by(7) {
            h.mark_failed(0x3000 + 4 * w);
            h.mark_failed(0x3000 + 4 * w);
        }
        let marked: Vec<u32> = (0..1024u32).filter(|w| h.failed(0x3000 + 4 * w)).collect();
        assert_eq!(marked, (0..1024u32).step_by(7).collect::<Vec<_>>());
        assert_eq!(h.pages.get(3).unwrap().n_failed as usize, marked.len());
    }

    #[test]
    fn forgetting_the_failures_of_a_page_leaves_its_counts_and_other_pages() {
        let mut h = HotTable::default();
        h.mark_failed(0x1004);
        h.mark_failed(0x2004);
        h.bump(0x1008);
        h.forget_failures(1);
        h.forget_failures(7); // a page never seen
        assert!(!h.failed(0x1004) && h.failed(0x2004));
        assert_eq!(h.count(0x1008), 1);
        assert_eq!(h.pages.get(1).unwrap().n_failed, 0);
        h.mark_failed(0x1004);
        assert!(h.failed(0x1004), "a forgotten pc can fail again");
    }

    #[test]
    fn nothing_allocates_after_a_pages_first_touch() {
        let mut h = HotTable::default();
        h.bump(0x1000);
        let probe = |h: &HotTable| (h.pages.buffers(), h.pages.get(1).unwrap().counts.as_ptr());
        let before = probe(&h);
        for i in 0..2000u32 {
            h.bump(0x1000 + 4 * (i % 1024));
            h.mark_failed(0x1000 + 4 * (i % 13));
            h.reset(0x1000 + 4 * (i % 5));
            h.forget_failures(1);
            assert!(!h.failed(0x1000));
        }
        assert_eq!(probe(&h), before);
    }
}
