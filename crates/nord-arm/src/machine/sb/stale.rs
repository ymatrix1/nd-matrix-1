//! OWNER: W1-CACHE. Invalidation of traces when code pages go stale (skeleton: drop every trace
//! that read the page; always correct, never revalidates).
//!
//! Spec (design note 6, WP W1-CACHE): the placeholder protocol of `take_real`/`absorb_pending`/
//! `put_back` and the first lines of `on_stale` stay as they are. `SbState::on_stale(page, mem)` is called for every page
//! `Mem::stale_code` yields (a store to a `HOOK_C` page, or a new `HOOK_X`). Revalidate: compare
//! each covering trace's `words` with memory; keep unchanged traces and re-mark `HOOK_C`
//! (`Mem::mark_code`); drop changed ones, reset links to them and release their backend code. A
//! page that now holds `HOOK_X` drops every trace. A page whose words really changed more than
//! `cfg.volatile_after` times becomes volatile (tier 1 only, `SbCache::mark_volatile`).

use super::uop::NO_SB;
use super::SbState;
use crate::mem::Mem;

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
        let _ = mem; // the skeleton does not revalidate
        for id in self.cache.covering(page) {
            let native = self.cache.get(id).native;
            if let Some(be) = self.backend.as_mut() {
                if native != NO_SB {
                    be.release(native);
                }
            }
            self.cache.kill(id);
            self.stats.dropped += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::SbConfig;

    #[test]
    fn every_trace_that_read_the_page_dies_and_the_others_live() {
        let mut st = SbState::default();
        let mut mem = Mem::new();
        let a = st.cache.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        let b = st
            .cache
            .insert(tiny_trace(0x2000, &[(0x2000, 1), (0x1008, 2)]));
        let c = st.cache.insert(tiny_trace(0x3000, &[(0x3000, 1)]));
        st.on_stale(1, &mut mem);
        assert_eq!(
            (
                st.cache.is_live(a),
                st.cache.is_live(b),
                st.cache.is_live(c)
            ),
            (false, false, true)
        );
        assert_eq!(st.stats.dropped, 2);
        st.on_stale(1, &mut mem); // nothing left to drop
        assert_eq!(st.stats.dropped, 2);
    }
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
        let mut slot = SbState {
            cfg: SbConfig::on(),
            ..SbState::default()
        };
        let id = slot.cache.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        let keep = slot.cache.insert(tiny_trace(0x3000, &[(0x3000, 1)]));
        let mut mem = Mem::new();
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
        let mut real = SbState::default();
        let id = real.cache.insert(tiny_trace(0x2000, &[(0x2000, 1)]));
        let mut ph = SbState::placeholder();
        let mut mem = Mem::new();
        ph.on_stale(2, &mut mem);
        real.absorb_pending(&mut ph, &mut mem);
        assert!(!real.cache.is_live(id) && ph.pending.is_empty());
    }
}
