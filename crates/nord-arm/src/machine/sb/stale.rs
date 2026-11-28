//! OWNER: W1-CACHE. Invalidation of traces when code pages go stale, and the flush of the whole
//! cache.
//!
//! Spec (design note 6, WP W1-CACHE): the placeholder protocol of `take_real`/`absorb_pending`/
//! `put_back` and the first lines of `on_stale` are as the skeleton had them. `on_stale(page,
//! mem)` is called for every page `Mem::stale_code` yields (a store to a `HOOK_C` page, or a new
//! `HOOK_X`). Revalidation: each trace that read `page` is compared word by word with memory
//! (`Sb::words`, only the words on that page); an unchanged trace stays and the page is flagged
//! `HOOK_C` again (`Mem::mark_code`), a changed one is dropped, its entry count reset and every
//! link to it cleared, and its backend code released. A page that now holds `HOOK_X` drops every
//! trace and is not re-flagged. A page whose words really changed more than `cfg.volatile_after`
//! times becomes volatile: it loses every trace and `SbCache::volatile` keeps the builder away
//! (tier 1 only). Reading memory never maps a page (I9): an unmapped word counts as changed.
//!
//! Nothing here allocates after a page's first insert, except the first push to `pending` of a
//! placeholder (design note, real-time rule).

use super::uop::{SbId, NO_SB};
use super::SbState;
use crate::mem::{Mem, HOOK_X};

// Mutation sites of W1-CACHE (block 650..=659 of the shared 600..=699 of `mutants.rs`). A site is
// `if is_active(<id>)`, a test-only branch that makes the code wrong; `mutants_are_killed` shows
// a test fails for each one. Ids: 650 revalidation accepts a changed word; 651 a page that keeps
// traces is not flagged `HOOK_C` again; 652 a hooked page keeps its unchanged traces; 653 the
// volatile rule fires one change early; 654 a dropped trace keeps its dispatch count; 655
// (cache.rs) a killed trace stays in the entry table.

/// Whether the test-only mutant `id` is active (`mutants::is_active`); always false outside tests.
#[cfg(test)]
#[inline(always)]
pub(super) fn is_active(id: u32) -> bool {
    super::mutants::is_active(id)
}

#[cfg(not(test))]
#[inline(always)]
pub(super) const fn is_active(_id: u32) -> bool {
    false
}

/// True when every word of `words` that lies on `page` still holds the value the trace was built
/// from. A word on an unmapped page (or straddling into one) counts as changed; nothing is mapped.
fn words_intact(mem: &mut Mem, words: &[(u32, u32)], page: u32) -> bool {
    if is_active(650) {
        return true;
    }
    words
        .iter()
        .filter(|(a, _)| a >> 12 == page)
        .all(|&(a, w)| mem.is_mapped(a) && mem.is_mapped(a.wrapping_add(3)) && mem.read32(a) == w)
}

impl SbState {
    /// The inert state `Machine::sb` holds while the real one is taken out.
    pub fn placeholder() -> Self {
        Self {
            placeholder: true,
            ..Self::default()
        }
    }

    /// Take the real state out of `slot` (`Machine::sb`), leaving a placeholder that records the
    /// pages going stale while the driver runs (a nested `Machine::run`, a hook, `inject_irq`
    /// reach `Machine::drop_stale_code`, which notifies whatever sits in the slot).
    pub fn take_real(slot: &mut SbState) -> SbState {
        std::mem::replace(slot, SbState::placeholder())
    }

    /// Forward the pages recorded by the placeholder in `slot` to `self`, then forget them.
    /// Allocation-free; the driver calls it before every drain and before `put_back`.
    pub fn absorb_pending(&mut self, slot: &mut SbState, mem: &mut Mem) {
        for &page in &slot.pending {
            self.on_stale(page, mem);
        }
        slot.pending.clear();
    }

    /// Put the real state back into `slot` after forwarding what the placeholder recorded.
    pub fn put_back(mut self, slot: &mut SbState, mem: &mut Mem) {
        self.absorb_pending(slot, mem);
        *slot = self;
    }

    /// React to `page` (a page number) having gone stale. On a placeholder only record it.
    pub fn on_stale(&mut self, page: u32, mem: &mut Mem) {
        if self.placeholder {
            if !self.pending.contains(&page) {
                self.pending.push(page);
            }
            return;
        }
        let base = page << 12;
        let hooked = mem.flags(base) & HOOK_X != 0 && !is_active(652);
        let dropped_before = self.stats.dropped;
        // Code that may have changed can be tried again where a build failed before.
        self.hot.forget_failures(page);
        // Walk the page's list from the back: dropping entry `i` only moves later entries.
        let mut changed = false;
        let mut i = self.cache.covering(page).len();
        while i > 0 {
            i -= 1;
            let id = self.cache.covering(page)[i];
            if hooked || !words_intact(mem, &self.cache.get(id).words, page) {
                changed |= !hooked;
                self.drop_one(id);
            }
        }
        if changed {
            let early = u8::from(is_active(653));
            if self.cache.note_change(page) > self.cfg.volatile_after.saturating_sub(early) {
                self.cache.mark_volatile(page);
            }
        }
        if self.cache.volatile(page) {
            let mut i = self.cache.covering(page).len();
            while i > 0 {
                i -= 1;
                let id = self.cache.covering(page)[i];
                self.drop_one(id);
            }
        }
        let kept = self.cache.covering(page).len();
        if kept > 0 {
            self.stats.revalidated += kept as u64;
            if mem.is_mapped(base) && !is_active(651) {
                mem.mark_code(base);
            }
        }
        if self.stats.dropped != dropped_before {
            self.cache.reset_dead_links();
        }
    }

    /// Drop trace `id`: release its backend code, kill it, forget its entry count. Links to it
    /// are left to `SbCache::reset_dead_links` (one pass for a batch). No-op for a dead id.
    fn drop_one(&mut self, id: SbId) {
        if !self.cache.is_live(id) {
            return;
        }
        let (entry, native) = {
            let sb = self.cache.get(id);
            (sb.entry, sb.native)
        };
        if native != NO_SB {
            if let Some(be) = self.backend.as_mut() {
                be.release(native);
            }
            self.cache.get_mut(id).native = NO_SB;
        }
        self.cache.kill(id);
        if !is_active(654) {
            self.hot.reset(entry);
        }
        self.stats.dropped += 1;
    }

    /// Drop trace `id` (a quarantined or superseded one): [`SbState::on_stale`]'s drop, plus the
    /// reset of every link to it.
    pub fn drop_trace(&mut self, id: SbId) {
        let before = self.stats.dropped;
        self.drop_one(id);
        if self.stats.dropped != before {
            self.cache.reset_dead_links();
        }
    }

    /// Drop every trace and release all backend code. Every `SbId` held anywhere is void
    /// afterwards (`SbCache::epoch` changed). Hotness and failure marks stay: the code did not
    /// change.
    pub fn flush_all(&mut self) {
        if let Some(be) = self.backend.as_mut() {
            for (_, sb) in self.cache.iter_live() {
                if sb.native != NO_SB {
                    be.release(sb.native);
                }
            }
        }
        self.cache.flush();
        self.stats.flushes += 1;
    }

    /// Flush when the cache is over `cfg.uop_cap` live uops (or holds too many dead slots);
    /// true when it did, and the caller must then forget every trace id it holds.
    pub fn flush_if_over_cap(&mut self) -> bool {
        let over = self.cache.over_cap(self.cfg.uop_cap);
        if over {
            self.flush_all();
        }
        over
    }
}

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod revalidate_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::SbConfig;

    #[test]
    fn a_placeholder_records_pages_once_and_touches_nothing_else() {
        let mut ph = SbState::placeholder();
        let mut mem = Mem::new();
        ph.on_stale(3, &mut mem);
        ph.on_stale(3, &mut mem);
        ph.on_stale(9, &mut mem);
        assert_eq!((ph.pending.clone(), ph.stats.dropped), (vec![3, 9], 0));
        assert!(ph.placeholder && !SbState::default().placeholder);
    }

    #[test]
    fn pages_that_go_stale_while_the_state_is_out_reach_it_when_it_is_put_back() {
        let mut mem = Mem::new();
        mem.write32(0x1000, 1);
        mem.write32(0x3000, 1);
        let mut slot = SbState {
            cfg: SbConfig::on(),
            ..SbState::default()
        };
        let id = slot.cache.insert(tiny_trace(0x1000, &[(0x1000, 99)]));
        let keep = slot.cache.insert(tiny_trace(0x3000, &[(0x3000, 1)]));
        let real = SbState::take_real(&mut slot);
        assert!(slot.placeholder && slot.cache.live_count() == 0 && real.cache.is_live(id));
        slot.on_stale(1, &mut mem); // a nested run drops stale code while the driver holds `real`
        assert!(real.cache.is_live(id), "not yet forwarded");
        real.put_back(&mut slot, &mut mem);
        assert!(!slot.placeholder && slot.pending.is_empty());
        assert!(!slot.cache.is_live(id) && slot.cache.is_live(keep));
        assert_eq!((slot.stats.dropped, slot.cfg.enabled), (1, true));
    }

    #[test]
    fn absorb_pending_forwards_and_clears() {
        let mut mem = Mem::new();
        mem.write32(0x2000, 1);
        let mut real = SbState::default();
        let id = real.cache.insert(tiny_trace(0x2000, &[(0x2000, 7)]));
        let mut ph = SbState::placeholder();
        ph.on_stale(2, &mut mem);
        real.absorb_pending(&mut ph, &mut mem);
        assert!(!real.cache.is_live(id) && ph.pending.is_empty());
    }

    #[test]
    fn a_state_with_nothing_cached_ignores_a_stale_page() {
        let mut st = SbState::default();
        let mut mem = Mem::new();
        st.on_stale(5, &mut mem);
        assert_eq!(
            (st.stats.dropped, st.stats.revalidated, st.pending.len()),
            (0, 0, 0)
        );
        assert_eq!(mem.flags(0x5000), 0, "no page is flagged for nothing");
    }
}
