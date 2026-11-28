//! OWNER: W1-CACHE. Dispatch counts that decide where a trace is built (skeleton: hash maps).
//!
//! Spec (design note 3.3, WP W1-CACHE): per page `Box<[u16; 1024]>` of dispatch counts plus a
//! per-page failure counter instead of the maps; the API below is frozen. `bump` is called at
//! dispatch points only (tier-1 slice boundaries and trace exits), so a loop of period `P` is
//! sampled at the pcs that are multiples of `slice` modulo `P`.

use std::collections::{HashMap, HashSet};

/// Hotness of code addresses.
#[derive(Default)]
pub struct HotTable {
    counts: HashMap<u32, u16>,
    failed: HashSet<u32>,
}

impl HotTable {
    /// Count one dispatch of `pc` (saturating) and return the new count.
    pub fn bump(&mut self, pc: u32) -> u16 {
        let c = self.counts.entry(pc).or_insert(0);
        *c = c.saturating_add(1);
        *c
    }

    /// Forget the count of `pc` (after a build, or when its trace dies).
    pub fn reset(&mut self, pc: u32) {
        self.counts.remove(&pc);
    }

    /// No trace can be built at `pc`: stop counting it (until the code changes).
    pub fn mark_failed(&mut self, pc: u32) {
        self.failed.insert(pc);
        self.counts.remove(&pc);
    }

    pub fn failed(&self, pc: u32) -> bool {
        self.failed.contains(&pc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_counts_per_pc_and_reset_forgets() {
        let mut h = HotTable::default();
        assert_eq!((h.bump(0x1000), h.bump(0x1000), h.bump(0x2000)), (1, 2, 1));
        h.reset(0x1000);
        assert_eq!(h.bump(0x1000), 1);
    }

    #[test]
    fn the_count_saturates_instead_of_wrapping() {
        let mut h = HotTable::default();
        h.counts.insert(0x1000, u16::MAX);
        assert_eq!(h.bump(0x1000), u16::MAX);
    }

    #[test]
    fn a_failed_pc_is_remembered_and_its_count_dropped() {
        let mut h = HotTable::default();
        h.bump(0x1000);
        h.mark_failed(0x1000);
        assert!(h.failed(0x1000) && !h.failed(0x1004));
        assert_eq!(h.bump(0x1000), 1, "the count restarted");
    }
}
