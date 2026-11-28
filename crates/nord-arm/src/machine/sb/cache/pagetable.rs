//! OWNER: W1-CACHE. Sparse directory from a page number to a per-page record, shared by the trace
//! cache (`cache.rs`) and the hot table (`hot.rs`).
//!
//! Two levels (1024 leaves of 1024 pages: the 4 GiB guest space of `mem.rs`), so an empty table
//! costs nothing, a lookup is two dependent loads plus the record, and a page that was touched
//! once is never allocated again. Records live in one `Vec` and never move or die while the table
//! lives, so a lookup after a page's first touch allocates nothing. Page numbers are `addr >> 12`
//! (below `1 << 20`); higher bits are ignored.

/// Pages per leaf, and leaves in the top table.
/// VERIFIED: `LEAF * TOP == 1 << (32 - mem::PAGE_SHIFT)`, the page count of the guest space in
/// `mem.rs`, checked by `the_directory_spans_the_whole_guest_space`.
const LEAF: usize = 1 << 10;
const TOP: usize = 1 << 10;

/// Page number to record of type `T`, created on demand.
pub(crate) struct PageTable<T> {
    /// Empty until the first record; then `TOP` entries, each a leaf of `slot + 1` (0 = none).
    top: Vec<Option<Box<[u32; LEAF]>>>,
    items: Vec<T>,
}

impl<T> Default for PageTable<T> {
    fn default() -> Self {
        Self {
            top: Vec::new(),
            items: Vec::new(),
        }
    }
}

impl<T> PageTable<T> {
    fn split(page: u32) -> (usize, usize) {
        let p = (page as usize) & (LEAF * TOP - 1);
        (p / LEAF, p % LEAF)
    }

    /// The record of `page`, when it has one.
    #[inline]
    pub fn get(&self, page: u32) -> Option<&T> {
        let (t, l) = Self::split(page);
        let slot = self.top.get(t)?.as_ref()?[l];
        self.items.get((slot as usize).checked_sub(1)?)
    }

    #[inline]
    pub fn get_mut(&mut self, page: u32) -> Option<&mut T> {
        let (t, l) = Self::split(page);
        let slot = self.top.get(t)?.as_ref()?[l];
        self.items.get_mut((slot as usize).checked_sub(1)?)
    }

    /// The record of `page`, made by `make` the first time. Allocates only then.
    pub fn get_or_insert_with(&mut self, page: u32, make: impl FnOnce() -> T) -> &mut T {
        let (t, l) = Self::split(page);
        if self.top.is_empty() {
            self.top.resize_with(TOP, || None);
        }
        let leaf = self.top[t].get_or_insert_with(|| Box::new([0u32; LEAF]));
        if leaf[l] == 0 {
            self.items.push(make());
            leaf[l] = self.items.len() as u32;
        }
        &mut self.items[leaf[l] as usize - 1]
    }

    /// Every record, in creation order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut()
    }

    /// Address and capacity of the buffers: two reads that are equal prove nothing was
    /// reallocated in between (the allocation tests of the cache and the hot table).
    #[cfg(test)]
    pub(crate) fn buffers(&self) -> [usize; 3] {
        [
            self.top.as_ptr() as usize,
            self.items.as_ptr() as usize,
            self.items.capacity(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_spans_the_whole_guest_space() {
        assert_eq!(LEAF * TOP, 1 << (32 - crate::mem::PAGE_SHIFT));
    }

    #[test]
    fn an_empty_table_allocates_nothing_and_finds_nothing() {
        let mut t = PageTable::<u8>::default();
        assert!(t.get(0).is_none() && t.get_mut(0xF_FFFF).is_none());
        assert_eq!((t.top.capacity(), t.buffers()[2]), (0, 0));
    }

    #[test]
    fn records_are_created_once_and_found_by_page() {
        let mut t = PageTable::<u32>::default();
        *t.get_or_insert_with(7, || 70) += 1;
        *t.get_or_insert_with(1 << 19, || 500) += 2;
        let again = t.get_or_insert_with(7, || unreachable!("made twice"));
        assert_eq!(*again, 71);
        assert_eq!(
            (t.get(7), t.get(1 << 19), t.get(8)),
            (Some(&71), Some(&502), None)
        );
        *t.get_mut(7).unwrap() = 9;
        assert_eq!(t.get(7), Some(&9));
        assert!(t.get_mut(9).is_none());
        let all: Vec<u32> = t.iter_mut().map(|v| *v).collect();
        assert_eq!(all, vec![9, 502], "creation order");
    }

    #[test]
    fn pages_of_one_leaf_and_of_two_leaves_do_not_alias() {
        let mut t = PageTable::<u32>::default();
        for p in [0u32, 1, 1023, 1024, 1025, 0xF_FFFF] {
            t.get_or_insert_with(p, || p + 1);
        }
        for p in [0u32, 1, 1023, 1024, 1025, 0xF_FFFF] {
            assert_eq!(t.get(p), Some(&(p + 1)), "page {p}");
        }
        assert!(t.get(2).is_none() && t.get(1026).is_none());
    }

    #[test]
    fn a_second_touch_of_a_page_does_not_allocate() {
        let mut t = PageTable::<[u8; 8]>::default();
        t.get_or_insert_with(3, || [0; 8]);
        let before = t.buffers();
        for _ in 0..100 {
            t.get_or_insert_with(3, || [1; 8])[0] += 1;
            assert!(t.get(3).is_some());
        }
        assert_eq!(t.buffers(), before);
        assert_eq!(t.get(3).unwrap()[0], 100);
    }
}
