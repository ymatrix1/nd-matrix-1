//! `uop_maintain`: revalidate, retranslate the dropped ranges, compact the tombstones.
//!
//! OWNER: W2-DRIVE. It runs at a pass boundary, only after a program change or an overlay load
//! (`lean_pass_uop` calls it when `uop_needs_maintenance()`), so it may allocate; `maintain_calls`
//! stays 0 in steady passes (gate G2). Three things to keep exact:
//!
//! * the ranges come from `hot.dropped` (one per dropped block, both PEy modes, so each appears
//!   twice) and are merged before translating, so the translator sees each range once;
//! * `hot.dropped` keeps its capacity: [`UopHot::install`] reserves room for every block so that
//!   `drop_block` never allocates (P14), and emptying the list must not give that room back;
//! * tombstones are compacted away once they outnumber the live blocks (a program change drops
//!   every block of the old program, and an engine that lives for hours would otherwise grow by
//!   a program per overlay load). Compaction renumbers the blocks, so the pc maps, the page lists
//!   and the links are rebuilt from the live blocks and the epoch moves.
use crate::core::uop::hot::UopHot;
use crate::core::uop::lower::{self, pcmap};
use crate::core::uop::run::CodePages;
use crate::core::uop::types::Link;
use crate::core::Core;

/// Tombstones needed before a compaction is considered. ASSUMED: a program change leaves a few
/// hundred; eight keeps a one-block overlay from rebuilding the maps. Recalibrate with
/// `uop-report` after an overlay trace.
const COMPACT_MIN_DEAD: usize = 8;

/// Merges overlapping or touching ranges, ascending. Sorts `v` in place.
fn merge(v: &mut [(u32, u32)]) -> Vec<(u32, u32)> {
    v.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(v.len());
    for &(lo, hi) in v.iter() {
        match out.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
            _ => out.push((lo, hi)),
        }
    }
    out
}

/// The merged ranges of `hot.dropped`, which is emptied but keeps its capacity (`drop_block` must
/// never allocate, so a `mem::take` or a `drain` into a new list is not an option).
pub(super) fn take_merged(hot: &mut UopHot) -> Vec<(u32, u32)> {
    let ranges = merge(&mut hot.dropped);
    hot.dropped.clear();
    ranges
}

/// Registers block `id` (whose uops are `hot.arena[b.first..]`) in the pc map of its mode and in
/// the lists of the code pages it touches, like [`UopHot::install`]: the last instruction may
/// reach two parcels past `last_pc`.
fn register(hot: &mut UopHot, id: u32) {
    let b = &hot.blocks[id as usize];
    let span = b.first as usize..(b.first + b.len) as usize;
    pcmap::install(&mut hot.pcmaps[usize::from(b.simd)], id, &hot.arena[span]);
    let first = CodePages::page_of(b.pc);
    let last = CodePages::page_of(b.last_pc + 2).or(first);
    if let (Some(p0), Some(p1)) = (first, last) {
        for list in &mut hot.page_blocks[p0..=p1] {
            list.push(id);
        }
    }
}

/// Drops the tombstones when they outnumber the live blocks (and are at least
/// [`COMPACT_MIN_DEAD`]). Returns the number removed.
pub(super) fn compact(hot: &mut UopHot) -> usize {
    let dead = hot.blocks.iter().filter(|b| b.len == 0).count();
    if dead < COMPACT_MIN_DEAD || dead * 2 <= hot.blocks.len() {
        return 0;
    }
    let old_blocks = std::mem::take(&mut hot.blocks);
    let old_arena = std::mem::take(&mut hot.arena);
    let live = old_blocks.iter().filter(|b| b.len != 0);
    hot.arena = Vec::with_capacity(live.clone().map(|b| b.len as usize).sum());
    hot.blocks = Vec::with_capacity(old_blocks.len() - dead);
    hot.page_blocks.iter_mut().for_each(Vec::clear);
    for mut b in old_blocks.into_iter().filter(|b| b.len != 0) {
        let span = b.first as usize..(b.first + b.len) as usize;
        b.first = hot.arena.len() as u32;
        b.links = [Link::NONE; 2];
        hot.arena.extend_from_slice(&old_arena[span]);
        let id = hot.blocks.len() as u32;
        hot.blocks.push(b);
        register(hot, id);
    }
    hot.epoch = hot.epoch.wrapping_add(1);
    dead
}

impl Core {
    /// Revalidates the dirty pages of the armed engine now (between blocks, outside a run).
    pub(super) fn uop_revalidate_now(&mut self) -> usize {
        let Some(mut hot) = self.uop.take() else {
            return 0;
        };
        let dropped = self.uop_revalidate_dirty(&mut hot);
        self.uop = Some(hot);
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
        for (lo, hi) in take_merged(&mut hot) {
            if let Ok((blocks, uops)) = lower::translate_range(self, &cfg, lo, hi) {
                self.uop_run.stats.translations += blocks.len() as u64;
                added += hot.install(blocks, uops).0 as usize;
            }
        }
        compact(&mut hot);
        self.uop = Some(hot);
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::drive::fixture::{arm_interp, nop_core};
    use crate::core::uop::verify::count_alloc::allocs_in;

    /// The three parcels of `R3 = R1 + R2`.
    const ADD: [u16; 3] = [0x013e, 0x0000, 0x1312];
    /// A valid one-parcel instruction that is not a NOP (`PROGRAM_POOL` of `verify::gen`).
    const OTHER: u16 = 0xc8be;

    /// Writes `parcels` at `at` one by one through the short-word alias window.
    fn alias(c: &mut Core, at: u32, parcels: &[u16]) {
        for (k, p) in parcels.iter().enumerate() {
            c.dm_write(at + k as u32, u32::from(*p)).unwrap();
        }
    }

    #[test]
    fn merge_joins_overlapping_and_touching_ranges() {
        assert_eq!(
            merge(&mut [(10, 20), (0, 4), (18, 30), (30, 31), (40, 41)]),
            [(0, 4), (10, 31), (40, 41)]
        );
        assert!(merge(&mut []).is_empty());
    }

    #[test]
    fn maintain_retranslates_the_dropped_range_and_the_new_code_runs_like_the_interpreter() {
        let mut want = nop_core(16);
        let mut got = nop_core(16);
        got.enable_uop(UopConfig::default(), &[(CODE, CODE + 16)])
            .unwrap();
        // three NOPs become `R3 = R1 + R2` in both cores; the engine core sees it as three writes
        // through the alias window
        crate::core::support::load(&mut want, CODE + 6, &ADD);
        alias(&mut got, CODE + 6, &ADD);
        for c in [&mut want, &mut got] {
            (c.r[1], c.r[2]) = (5, 6);
        }
        assert!(got.uop_needs_maintenance());
        let before = got.uop_stats().translations;
        let added = got.uop_maintain();
        assert!(added > 0, "the dropped range was translated again");
        assert_eq!(got.uop_stats().translations, before + added as u64);
        assert!(
            !got.uop_needs_maintenance(),
            "dirty and dropped are both empty"
        );
        assert_eq!((want.run(16), got.run_uop(16)), (None, None));
        assert_eq!(want.state_mismatch(&got), None);
        assert_eq!(got.r[3], 11, "the add ran, not the three NOPs it replaced");
        assert!(
            got.uop_stats().steps_uop > 0,
            "through blocks: {:?}",
            got.uop_stats()
        );
    }

    #[test]
    fn maintain_translates_with_the_stop_pc_and_keeps_the_dropped_capacity() {
        let mut c = nop_core(24);
        c.uop_set_stop_pc(Some(CODE + 12));
        arm_interp(&mut c, &[(CODE, CODE + 24)]);
        alias(&mut c, CODE + 14, &[0x0001]); // not a change: nothing is dropped
        assert_eq!(c.uop_maintain(), 0);
        alias(&mut c, CODE + 14, &[OTHER]);
        assert!(c.uop_maintain() > 0);
        let hot = c.uop.as_ref().unwrap();
        assert!(hot.dropped.is_empty());
        assert!(
            hot.dropped.capacity() >= hot.blocks.len(),
            "drop_block must still never allocate: {} < {}",
            hot.dropped.capacity(),
            hot.blocks.len()
        );
        for simd in [false, true] {
            let (id, idx) = hot.pcmap(simd).get(CODE + 12).expect("mapped");
            assert_eq!(
                (idx, hot.blocks[id as usize].pc),
                (0, CODE + 12),
                "mode {simd}"
            );
        }
        // dropping a block of the fresh table allocates nothing (P14, after maintain too)
        let mut hot = c.uop.take().unwrap();
        let id = hot.pcmap(false).get(CODE).unwrap().0;
        let (dropped, allocs) = allocs_in(|| hot.drop_block(id));
        assert_eq!((dropped, allocs), (true, 0));
    }

    #[test]
    fn tombstones_are_compacted_and_the_engine_keeps_equalling_the_interpreter() {
        // sixteen two-instruction ranges 32 parcels apart, rewritten over and over: without
        // compaction the block list and the arena would grow by thirty-two blocks per round. (A
        // dropped range is `(pc, last_pc + 3)`, so each retranslation widens a block by two
        // parcels; the spacing keeps the blocks of the rounds from merging.)
        let ranges: Vec<(u32, u32)> = (0..16)
            .map(|k| (CODE + 32 * k, CODE + 32 * k + 2))
            .collect();
        let mut got = nop_core(0x200);
        got.enable_uop(UopConfig::default(), &ranges).unwrap();
        let live = got.uop.as_ref().unwrap().blocks.len();
        assert_eq!(live, 32);
        let mut want = nop_core(0x200);
        for round in 0..6u16 {
            let w = if round % 2 == 0 { OTHER } else { 0x0001 };
            for k in 0..16 {
                alias(&mut got, CODE + 32 * k, &[w]);
                crate::core::support::load(&mut want, CODE + 32 * k, &[w]);
            }
            assert!(got.uop_maintain() > 0, "round {round}");
            let hot = got.uop.as_ref().unwrap();
            assert!(
                hot.blocks.len() <= 2 * live + COMPACT_MIN_DEAD,
                "round {round}: {} blocks",
                hot.blocks.len()
            );
            // the dead blocks are older and shorter than the live ones, so the arena stays below
            // twice what the live blocks need
            let live_uops: usize = hot.blocks.iter().map(|b| b.len as usize).sum();
            assert!(
                hot.arena.len() <= 2 * live_uops,
                "round {round}: {} uops for {live_uops} live",
                hot.arena.len()
            );
        }
        // the last round restored the NOPs: both cores hold the same code and run it alike
        assert_eq!((want.run(40), got.run_uop(40)), (None, None));
        assert_eq!(want.state_mismatch(&got), None);
        assert!(got.uop_stats().steps_uop > 0);
        // the last round compacted: no tombstone and no hole in the arena are left, every block
        // is reachable through the map and registered on its page
        let hot = got.uop.as_ref().unwrap();
        for (id, b) in hot.blocks.iter().enumerate() {
            assert_ne!(b.len, 0, "no tombstone left after the last compaction");
            assert!(hot.page_blocks[CodePages::page_of(b.pc).unwrap()].contains(&(id as u32)));
        }
        let live_uops: usize = hot.blocks.iter().map(|b| b.len as usize).sum();
        assert_eq!(hot.arena.len(), live_uops, "compaction rebuilt the arena");
        assert_eq!(hot.pcmap(false).get(CODE + 32).map(|e| e.1), Some(0));
    }

    #[test]
    fn compaction_leaves_a_table_with_few_tombstones_alone() {
        let mut hot = crate::core::uop::drive::fixture::interp_block(4).0;
        assert_eq!(compact(&mut hot), 0);
        let mut c = core_with(&[0x0001; 8]);
        arm_interp(&mut c, &[(CODE, CODE + 8)]);
        let mut hot = c.uop.take().unwrap();
        assert!(hot.drop_block(0) && hot.drop_block(1));
        assert_eq!(
            compact(&mut hot),
            0,
            "two tombstones are below the threshold"
        );
        assert_eq!(hot.blocks.len(), 2);
    }
}
