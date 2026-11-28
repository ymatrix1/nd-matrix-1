//! OWNER: W1-CACHE. The trace cache: an arena of traces found through per-page word tables.
//!
//! Spec (design note 3.3, WP W1-CACHE): arena `Vec<Sb>`; per page `at: Box<[u32; 1024]>` (word to
//! `SbId + 1`) and the list of live traces that read the page, a `modified` counter and the
//! `volatile` flag; `uops_total` against `SbConfig::uop_cap`; `epoch`. `lookup`, `covering` and
//! `kill` allocate nothing, and `insert` allocates only the first time a page is seen (and when
//! the arena or a page list grows). A dead trace keeps its slot, so an id never names another
//! trace until [`SbCache::flush`]; its uops, exits and words are freed.
//!
//! Link safety: an `Exit::link` or inline-cache entry may name a dead trace. Callers never
//! follow one without [`SbCache::is_live`], and [`SbCache::reset_dead_links`] clears them in bulk
//! (the stale path does it after every drop). `flush` restarts the ids, so every holder of an
//! `SbId` (links, backend ids, a driver local) must be forgotten with it; [`SbCache::epoch`]
//! changes on every flush for the holders that cannot be enumerated. `Clone` of the owning
//! `SbState` yields an empty cache, so this type is `Default` but not `Clone`.

pub(super) mod pagetable;

use super::stale::is_active;
use super::uop::{Sb, SbId, NO_SB};
use pagetable::PageTable;

/// Instruction words per 4 KiB page, the size of a per-page word table.
/// VERIFIED: `mem::PAGE_SIZE / 4`, checked by `a_page_has_one_table_slot_per_word`.
const WORDS: usize = 1 << 10;

/// Arena slots (live and dead) after which [`SbCache::over_cap`] asks for a flush, so slots of
/// dropped traces cannot pile up without bound under code that keeps changing. ASSUMED: about
/// 90 bytes per slot, 12 MB at the cap; the heavy stretch builds a few thousand traces
/// (design note 3.3), so 131072 is never reached without churn. A counter run would confirm it.
pub const SLOT_CAP: usize = 1 << 17;

/// What the cache knows about one 4 KiB page.
struct PageData {
    /// Word index to `SbId + 1` of the live trace entered there (0 = none).
    at: Box<[u32; WORDS]>,
    /// Live traces that read a word of the page, in insertion order.
    traces: Vec<SbId>,
    /// Stale events of the page that really changed a word a trace read (saturating).
    modified: u8,
    volatile: bool,
}

impl PageData {
    fn new() -> Self {
        Self {
            at: Box::new([0; WORDS]),
            traces: Vec::new(),
            modified: 0,
            volatile: false,
        }
    }
}

/// Traces by arena id, looked up by entry pc.
#[derive(Default)]
pub struct SbCache {
    arena: Vec<Sb>,
    pages: PageTable<PageData>,
    uops: usize,
    live: usize,
    epoch: u32,
}

#[inline(always)]
fn word_index(pc: u32) -> usize {
    (pc >> 2) as usize & (WORDS - 1)
}

impl SbCache {
    fn page_mut(&mut self, page: u32) -> &mut PageData {
        self.pages.get_or_insert_with(page, PageData::new)
    }

    /// The live trace that starts at `pc` (never a misaligned one: only ARM code is traced).
    #[inline]
    pub fn lookup(&self, pc: u32) -> Option<SbId> {
        if pc & 3 != 0 {
            return None;
        }
        self.pages.get(pc >> 12)?.at[word_index(pc)].checked_sub(1)
    }

    pub fn get(&self, id: SbId) -> &Sb {
        &self.arena[id as usize]
    }

    pub fn get_mut(&mut self, id: SbId) -> &mut Sb {
        &mut self.arena[id as usize]
    }

    /// Add a trace; a live trace with the same entry pc is replaced (killed). A trace whose
    /// entry is not word aligned is kept and tracked for staleness but never found by `lookup`.
    pub fn insert(&mut self, mut sb: Sb) -> SbId {
        if let Some(old) = self.lookup(sb.entry) {
            self.kill(old);
        }
        sb.live = true;
        let id = self.arena.len() as SbId;
        self.uops += sb.uops.len();
        self.live += 1;
        for &page in &sb.pages[..sb.n_pages as usize] {
            self.page_mut(page).traces.push(id);
        }
        if sb.entry & 3 == 0 {
            self.page_mut(sb.entry >> 12).at[word_index(sb.entry)] = id + 1;
        }
        self.arena.push(sb);
        id
    }

    /// True for a live trace; `NO_SB` and ids past the arena are not live.
    #[inline]
    pub fn is_live(&self, id: SbId) -> bool {
        self.arena.get(id as usize).is_some_and(|sb| sb.live)
    }

    pub fn live_count(&self) -> usize {
        self.live
    }

    /// Uops held by live traces (the cap of `SbConfig::uop_cap` applies to this).
    pub fn uops_total(&self) -> usize {
        self.uops
    }

    /// Arena slots in use, dead ones included.
    pub fn slots(&self) -> usize {
        self.arena.len()
    }

    /// True when the live uops exceed `uop_cap` or the arena holds more than [`SLOT_CAP`] slots:
    /// the driver then flushes (`SbState::flush_if_over_cap`).
    pub fn over_cap(&self, uop_cap: usize) -> bool {
        self.uops > uop_cap || self.arena.len() > SLOT_CAP
    }

    /// Number of flushes so far (wrapping): ids obtained before a change are void.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Drop every trace (ids restart; the caller must also forget every link and backend id).
    /// Volatile marks and change counters survive: they describe the code, not the cache.
    pub fn flush(&mut self) {
        self.arena.clear();
        for p in self.pages.iter_mut() {
            p.at.fill(0);
            p.traces.clear();
        }
        self.uops = 0;
        self.live = 0;
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// True when the code of `page` (a page number) changed too often to be traced.
    pub fn volatile(&self, page: u32) -> bool {
        self.pages.get(page).is_some_and(|p| p.volatile)
    }

    pub fn mark_volatile(&mut self, page: u32) {
        self.page_mut(page).volatile = true;
    }

    /// Count one stale event of `page` that really changed a word a trace read; returns the
    /// count so far (saturating at 255).
    pub fn note_change(&mut self, page: u32) -> u8 {
        let p = self.page_mut(page);
        p.modified = p.modified.saturating_add(1);
        p.modified
    }

    /// Mark trace `id` dead and free its bodies; the slot stays so ids and links stay valid.
    /// Links that name it are left to [`SbCache::reset_dead_links`]; an unknown id is ignored.
    pub fn kill(&mut self, id: SbId) {
        let Some(sb) = self.arena.get_mut(id as usize) else {
            return;
        };
        if !sb.live {
            return;
        }
        sb.live = false;
        let (entry, pages, n_pages, n_uops) =
            (sb.entry, sb.pages, sb.n_pages as usize, sb.uops.len());
        sb.uops = Box::default();
        sb.exits = Box::default();
        sb.words = Box::default();
        self.uops -= n_uops;
        self.live -= 1;
        if entry & 3 == 0 && !is_active(655) {
            if let Some(p) = self.pages.get_mut(entry >> 12) {
                let slot = &mut p.at[word_index(entry)];
                if *slot == id + 1 {
                    *slot = 0;
                }
            }
        }
        for &page in &pages[..n_pages] {
            if let Some(p) = self.pages.get_mut(page) {
                if let Some(at) = p.traces.iter().position(|t| *t == id) {
                    p.traces.remove(at);
                }
            }
        }
    }

    /// Live traces that read a word of `page` (a page number), in insertion order. Killing the
    /// trace at index `i` of this list only moves entries after `i`, so a caller may walk it
    /// from the back and kill as it goes (the stale path does).
    #[inline]
    pub fn covering(&self, page: u32) -> &[SbId] {
        self.pages.get(page).map_or(&[], |p| &p.traces)
    }

    /// Every live trace with its id, in id order.
    pub fn iter_live(&self) -> impl Iterator<Item = (SbId, &Sb)> {
        self.arena
            .iter()
            .enumerate()
            .filter(|(_, sb)| sb.live)
            .map(|(i, sb)| (i as SbId, sb))
    }

    fn names_dead(&self, id: SbId) -> bool {
        id != NO_SB && !self.is_live(id)
    }

    /// Clear every link and inline-cache entry of a live trace that names a dead one (they
    /// become unlinked, so the driver looks the successor up again). Returns how many it reset.
    pub fn reset_dead_links(&mut self) -> usize {
        let mut reset = 0;
        for i in 0..self.arena.len() {
            if !self.arena[i].live {
                continue;
            }
            for e in 0..self.arena[i].exits.len() {
                let x = &self.arena[i].exits[e];
                let dead = [
                    self.names_dead(x.link),
                    self.names_dead(x.ic_id[0]),
                    self.names_dead(x.ic_id[1]),
                ];
                if dead == [false; 3] {
                    continue;
                }
                let x = &mut self.arena[i].exits[e];
                if dead[0] {
                    x.link = NO_SB;
                }
                if dead[1] {
                    (x.ic_id[0], x.ic_pc[0]) = (NO_SB, 0);
                }
                if dead[2] {
                    (x.ic_id[1], x.ic_pc[1]) = (NO_SB, 0);
                }
                reset += dead.iter().filter(|d| **d).count();
            }
        }
        reset
    }
}

/// A one-uop trace for tests of the cache, the stale path and the driver.
#[cfg(test)]
pub(crate) fn tiny_trace(entry: u32, words: &[(u32, u32)]) -> Sb {
    use super::uop::{Exit, ExitKind, Uop};
    let uops = vec![Uop::generic_end(0xEAFF_FFFE, entry, 0)];
    let exits = vec![Exit::new(ExitKind::Dynamic, 1, entry, 0)];
    Sb::from_parts(entry, 1, uops, exits, words.to_vec()).unwrap()
}

#[cfg(test)]
mod behaviour_tests;

#[cfg(test)]
mod model_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_has_one_table_slot_per_word() {
        assert_eq!(WORDS * 4, crate::mem::PAGE_SIZE);
    }

    #[test]
    fn insert_lookup_and_get_round_trip() {
        let mut c = SbCache::default();
        let id = c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        assert_eq!((c.lookup(0x1000), c.lookup(0x1004)), (Some(id), None));
        assert_eq!((c.lookup(0x2000), c.lookup(0x1001)), (None, None));
        assert_eq!(c.get(id).entry, 0x1000);
        c.get_mut(id).enters += 1;
        assert_eq!(
            (c.get(id).enters, c.live_count(), c.uops_total(), c.slots()),
            (1, 1, 1, 1)
        );
        assert!(c.is_live(id) && !c.is_live(id + 1) && !c.is_live(NO_SB));
    }

    #[test]
    fn a_killed_trace_is_dead_empty_and_gone_from_the_lookup() {
        let mut c = SbCache::default();
        let a = c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        let b = c.insert(tiny_trace(0x2000, &[(0x2000, 1)]));
        c.kill(a);
        c.kill(a); // idempotent
        c.kill(99); // unknown ids are ignored
        assert_eq!((c.lookup(0x1000), c.lookup(0x2000)), (None, Some(b)));
        assert_eq!(
            (c.is_live(a), c.is_live(b), c.live_count(), c.uops_total()),
            (false, true, 1, 1)
        );
        let dead = c.get(a);
        assert!(dead.uops.is_empty() && dead.words.is_empty() && dead.exits.is_empty());
        assert_eq!(c.slots(), 2, "the slot stays, ids are stable");
    }
}
