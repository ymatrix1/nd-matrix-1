//! OWNER: W1-CACHE. The trace cache (skeleton: a correct but unoptimised arena with a hash map
//! from entry pc to trace).
//!
//! Spec (design note 3.3, WP W1-CACHE): arena `Vec<Sb>`; `at: per page Box<[u32; 1024]>` (word
//! to `SbId + 1`) instead of the hash map; `by_page: Vec<Vec<SbId>>`; link-safe liveness (an
//! `Exit::link` or inline-cache entry to a dead trace is never followed: callers test
//! [`SbCache::is_live`]); a per-page `modified` counter for the volatile rule; `uops_total`
//! against `SbConfig::uop_cap` (flush all on overflow, done by the driver); `epoch`. A dead trace
//! keeps its slot (ids stay stable) but frees its uops, exits and words. `Clone` of the owning
//! `SbState` yields an empty cache, so this type is `Default` but not `Clone`.

use super::uop::{Sb, SbId};
use std::collections::{HashMap, HashSet};

/// Traces by arena id, looked up by entry pc.
#[derive(Default)]
pub struct SbCache {
    arena: Vec<Sb>,
    /// Entry pc to the live trace starting there.
    at: HashMap<u32, SbId>,
    uops: usize,
    live: usize,
    volatile: HashSet<u32>,
}

impl SbCache {
    /// The live trace that starts at `pc`.
    pub fn lookup(&self, pc: u32) -> Option<SbId> {
        self.at.get(&pc).copied()
    }

    pub fn get(&self, id: SbId) -> &Sb {
        &self.arena[id as usize]
    }

    pub fn get_mut(&mut self, id: SbId) -> &mut Sb {
        &mut self.arena[id as usize]
    }

    /// Add a trace; a live trace with the same entry pc is replaced (killed).
    pub fn insert(&mut self, sb: Sb) -> SbId {
        if let Some(old) = self.lookup(sb.entry) {
            self.kill(old);
        }
        let id = self.arena.len() as SbId;
        self.uops += sb.uops.len();
        self.live += 1;
        self.at.insert(sb.entry, id);
        self.arena.push(sb);
        id
    }

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

    /// Drop every trace (ids restart; the caller must also forget every link and backend id).
    pub fn flush(&mut self) {
        self.arena.clear();
        self.at.clear();
        self.uops = 0;
        self.live = 0;
    }

    /// True when the code of `page` (a page number) changed too often to be traced.
    pub fn volatile(&self, page: u32) -> bool {
        self.volatile.contains(&page)
    }

    pub fn mark_volatile(&mut self, page: u32) {
        self.volatile.insert(page);
    }

    /// Mark trace `id` dead and free its bodies; the slot stays so ids and links stay valid.
    pub fn kill(&mut self, id: SbId) {
        let sb = &mut self.arena[id as usize];
        if !sb.live {
            return;
        }
        sb.live = false;
        self.uops -= sb.uops.len();
        self.live -= 1;
        if self.at.get(&sb.entry) == Some(&id) {
            self.at.remove(&sb.entry);
        }
        sb.uops = Box::default();
        sb.exits = Box::default();
        sb.words = Box::default();
    }

    /// Live traces that read a word of `page` (a page number).
    pub fn covering(&self, page: u32) -> Vec<SbId> {
        (0..self.arena.len() as SbId)
            .filter(|id| {
                let sb = &self.arena[*id as usize];
                sb.live && sb.covers_page(page)
            })
            .collect()
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
mod tests {
    use super::*;

    #[test]
    fn insert_lookup_and_get_round_trip() {
        let mut c = SbCache::default();
        let id = c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        assert_eq!((c.lookup(0x1000), c.lookup(0x1004)), (Some(id), None));
        assert_eq!(c.get(id).entry, 0x1000);
        c.get_mut(id).enters += 1;
        assert_eq!(
            (c.get(id).enters, c.live_count(), c.uops_total()),
            (1, 1, 1)
        );
        assert!(c.is_live(id) && !c.is_live(id + 1));
    }

    #[test]
    fn a_killed_trace_is_dead_empty_and_gone_from_the_lookup() {
        let mut c = SbCache::default();
        let a = c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        let b = c.insert(tiny_trace(0x2000, &[(0x2000, 1)]));
        c.kill(a);
        c.kill(a); // idempotent
        assert_eq!((c.lookup(0x1000), c.lookup(0x2000)), (None, Some(b)));
        assert_eq!(
            (c.is_live(a), c.is_live(b), c.live_count(), c.uops_total()),
            (false, true, 1, 1)
        );
        assert!(c.get(a).uops.is_empty() && c.get(a).words.is_empty());
    }

    #[test]
    fn a_second_trace_at_the_same_entry_replaces_the_first() {
        let mut c = SbCache::default();
        let a = c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        let b = c.insert(tiny_trace(0x1000, &[(0x1000, 2)]));
        assert_ne!(a, b);
        assert_eq!(
            (c.lookup(0x1000), c.is_live(a), c.live_count()),
            (Some(b), false, 1)
        );
    }

    #[test]
    fn covering_lists_the_live_traces_that_read_a_page() {
        let mut c = SbCache::default();
        let a = c.insert(tiny_trace(0x1000, &[(0x1000, 1), (0x5000, 2)]));
        let b = c.insert(tiny_trace(0x2000, &[(0x2000, 1)]));
        assert_eq!(
            (c.covering(1), c.covering(5), c.covering(2)),
            (vec![a], vec![a], vec![b])
        );
        c.kill(a);
        assert!(c.covering(5).is_empty());
    }

    #[test]
    fn flush_empties_the_cache_and_volatile_pages_are_remembered() {
        let mut c = SbCache::default();
        c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        c.mark_volatile(3);
        c.flush();
        assert_eq!(
            (c.live_count(), c.uops_total(), c.lookup(0x1000)),
            (0, 0, None)
        );
        assert!(c.volatile(3) && !c.volatile(4));
    }
}
