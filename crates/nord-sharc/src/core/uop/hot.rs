//! The boxed hot state of the engine: the uop arena, the blocks, the pc map and the handler
//! table. `Core::uop_try` takes it out of the core (`self.uop.take()`), runs, and puts it back,
//! so the `&mut Core` and the `&UopHot` of a running block never overlap.
//!
//! OWNER: W0 (skeleton); contract changes go back to W0. Handler registration is STATIC: the
//! table is built here by calling the five `register` functions in a fixed order, each of which
//! may only write inside its own range of `UopKind` (test `registration_stays_in_family_ranges`),
//! so work packages never edit this file.
use super::exec::{h_core, h_fixed, h_float, h_flow, h_move};
use super::run::CodePages;
use super::types::{bail_handler, Block, Handler, KindFamily, Uop, NONE};
use crate::core::SRAM_BYTES;

/// Short words addressable in the flat SRAM image (the pc range a block can start in).
pub const PCMAP_SPAN: u32 = (SRAM_BYTES / 2) as u32;
const LEAF: usize = 256;
const LEAF_SHIFT: u32 = 8;
const EMPTY: (u32, u32) = (NONE, NONE);
/// One 256-entry leaf of the [`PcMap`].
type Leaf = Box<[(u32, u32); LEAF]>;

/// Maps a pc to `(block id, uop index)` of the uop that starts there, so a block can be entered
/// at any uop boundary. Two levels: a table of lazily allocated 256-entry leaves.
pub(crate) struct PcMap {
    top: Vec<Option<Leaf>>,
}

impl Default for PcMap {
    fn default() -> Self {
        Self::new()
    }
}

impl PcMap {
    /// An empty map (only the top table is allocated).
    pub fn new() -> Self {
        PcMap {
            top: vec![None; (PCMAP_SPAN >> LEAF_SHIFT) as usize],
        }
    }

    /// The uop that starts at `pc`, `None` when no block covers it.
    #[inline]
    pub fn get(&self, pc: u32) -> Option<(u32, u32)> {
        let leaf = self.top.get((pc >> LEAF_SHIFT) as usize)?.as_deref()?;
        let e = leaf[pc as usize & (LEAF - 1)];
        (e != EMPTY).then_some(e)
    }

    /// Records that the uop `v = (block, index)` starts at `pc`. A pc outside the SRAM image is
    /// ignored.
    pub fn set(&mut self, pc: u32, v: (u32, u32)) {
        if let Some(slot) = self.top.get_mut((pc >> LEAF_SHIFT) as usize) {
            let leaf = slot.get_or_insert_with(|| Box::new([EMPTY; LEAF]));
            leaf[pc as usize & (LEAF - 1)] = v;
        }
    }

    /// Forgets every entry whose pc lies in `lo..hi`.
    pub fn clear_range(&mut self, lo: u32, hi: u32) {
        let hi = hi.min(PCMAP_SPAN);
        if lo >= hi {
            return;
        }
        for t in (lo >> LEAF_SHIFT)..=((hi - 1) >> LEAF_SHIFT) {
            let Some(Some(leaf)) = self.top.get_mut(t as usize) else {
                continue;
            };
            let base = t << LEAF_SHIFT;
            for pc in lo.max(base)..hi.min(base + LEAF as u32) {
                leaf[(pc - base) as usize] = EMPTY;
            }
        }
    }

    /// Number of leaves allocated (for tests and reports).
    pub fn leaves(&self) -> usize {
        self.top.iter().flatten().count()
    }
}

/// Blocks, uops and tables of an enabled engine.
pub(crate) struct UopHot {
    /// All uops of all blocks; `Block::first..first + len` indexes it.
    pub arena: Vec<Uop>,
    pub blocks: Vec<Block>,
    /// One pc map per SIMD mode (`[PEy off, PEy on]`): the mode is baked into a block, and the
    /// ORG Core2 pass runs about 94% of its steps with PEy on (`sharc-jit-ceiling-study.md`,
    /// `simd=5584.0` of 5953), so every range is translated for both modes and the entry looks
    /// the pc up in the map of the current `core.simd()` (design 7.1, W0 amendment).
    pub pcmaps: [PcMap; 2],
    /// One entry per `UopKind`, [`super::types::UopKind::MAX`] slots.
    pub handlers: Box<[Handler]>,
    /// Block ids per code page, for revalidation after a write.
    pub page_blocks: Vec<Vec<u32>>,
    /// Bumped whenever blocks are dropped, so cached `Link`s revalidate on use.
    pub epoch: u32,
    /// Ranges `(lo, hi)` whose blocks were dropped by revalidation; `uop_maintain` retranslates
    /// them (W0 amendment: the design left no place to record them).
    pub dropped: Vec<(u32, u32)>,
    #[cfg(feature = "uop-native")]
    pub native: Option<Box<super::native::NativeTable>>,
}

impl Default for UopHot {
    fn default() -> Self {
        Self::new()
    }
}

impl UopHot {
    /// An engine state without blocks and with the registered handler table.
    pub fn new() -> Self {
        UopHot {
            arena: Vec::new(),
            blocks: Vec::new(),
            pcmaps: [PcMap::new(), PcMap::new()],
            handlers: Self::handler_table(),
            page_blocks: vec![Vec::new(); super::run::CODE_PAGES],
            epoch: 0,
            dropped: Vec::new(),
            #[cfg(feature = "uop-native")]
            native: None,
        }
    }

    /// The pc map of SIMD mode `simd` (PEy on when true).
    #[inline]
    pub fn pcmap(&self, simd: bool) -> &PcMap {
        &self.pcmaps[usize::from(simd)]
    }

    /// The static handler table: every slot starts as [`bail_handler`] and each family registers
    /// its own range, in this order.
    pub fn handler_table() -> Box<[Handler]> {
        let mut t: Vec<Handler> =
            vec![bail_handler as Handler; usize::from(super::types::UopKind::MAX)];
        h_core::register(&mut t);
        h_float::register(&mut t);
        h_fixed::register(&mut t);
        h_move::register(&mut t);
        h_flow::register(&mut t);
        t.into_boxed_slice()
    }

    /// Appends translated blocks (`Block::first` relative to `uops`) to the arena, assigns ids,
    /// enters their uop boundaries into the pc map and their pages into `page_blocks`. Returns the
    /// number of blocks and uops added.
    pub fn install(&mut self, blocks: Vec<Block>, uops: Vec<Uop>) -> (u32, u32) {
        let base = self.arena.len() as u32;
        let added = (blocks.len() as u32, uops.len() as u32);
        // `drop_block` runs on the audio path (a write into a code page): its push must never
        // allocate, so the list always has room for every block that could still be dropped
        // (`dropped` is emptied by `uop_maintain`, which reserves again through `install`).
        let want = (self.blocks.len() + blocks.len()).saturating_sub(self.dropped.len());
        self.dropped.reserve(want);
        self.arena.extend(uops);
        for mut b in blocks {
            b.first += base;
            let id = self.blocks.len() as u32;
            let span = b.first as usize..(b.first + b.len) as usize;
            super::lower::pcmap::install(
                &mut self.pcmaps[usize::from(b.simd)],
                id,
                &self.arena[span],
            );
            // the last instruction may reach two parcels past `last_pc`
            let first = CodePages::page_of(b.pc);
            let last = CodePages::page_of(b.last_pc + 2).or(first);
            if let (Some(p0), Some(p1)) = (first, last) {
                for p in p0..=p1 {
                    self.page_blocks[p].push(id);
                }
            }
            self.blocks.push(b);
        }
        added
    }

    /// Tombstones block `id`: clears its pc-map entries, remembers its range for retranslation
    /// and bumps the epoch so cached links revalidate. Returns false when it was already dropped.
    pub fn drop_block(&mut self, id: u32) -> bool {
        let Some(b) = self.blocks.get_mut(id as usize) else {
            return false;
        };
        if b.len == 0 {
            return false;
        }
        let map = &mut self.pcmaps[usize::from(b.simd)];
        for u in &self.arena[b.first as usize..(b.first + b.len) as usize] {
            map.clear_range(u.pc, u.pc + 1);
        }
        self.dropped.push((b.pc, b.last_pc + 3));
        (b.len, b.steps, b.poisoned) = (0, 0, true);
        self.epoch = self.epoch.wrapping_add(1);
        true
    }

    /// The registration function of `family`, for the range test.
    pub fn register_fn(family: KindFamily) -> fn(&mut [Handler]) {
        match family {
            KindFamily::Core => h_core::register,
            KindFamily::Float => h_float::register,
            KindFamily::Fixed => h_fixed::register,
            KindFamily::Move => h_move::register,
            KindFamily::Flow => h_flow::register,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::UopKind;

    fn addr(h: Handler) -> usize {
        h as usize
    }

    #[test]
    fn registration_stays_in_family_ranges() {
        let default = addr(bail_handler);
        for family in KindFamily::ALL {
            let mut t: Vec<Handler> = vec![bail_handler as Handler; usize::from(UopKind::MAX)];
            UopHot::register_fn(family)(&mut t);
            for (i, h) in t.iter().enumerate() {
                if addr(*h) != default {
                    assert!(
                        family.range().contains(&i),
                        "{family:?} registered slot {i:#x} outside {:?}",
                        family.range()
                    );
                }
            }
        }
    }

    #[test]
    fn a_new_engine_state_is_empty_and_every_slot_has_a_handler() {
        let h = UopHot::new();
        assert!(h.arena.is_empty() && h.blocks.is_empty() && h.dropped.is_empty());
        assert_eq!(h.handlers.len(), usize::from(UopKind::MAX));
        assert_eq!(h.page_blocks.len(), crate::core::uop::run::CODE_PAGES);
        assert_eq!((h.pcmaps[0].leaves(), h.pcmaps[1].leaves()), (0, 0));
        // the BAIL slot is the explicit bail handler; unknown slots fall back the same way
        assert_eq!(
            addr(h.handlers[usize::from(UopKind::BAIL.0)]),
            addr(bail_handler)
        );
    }

    #[test]
    fn pcmap_round_trips_clears_ranges_and_ignores_pcs_past_the_image() {
        let mut m = PcMap::new();
        assert_eq!(m.get(0x12_0000), None);
        m.set(0x12_0000, (3, 0));
        m.set(0x12_0001, (3, 1));
        m.set(0x12_0100, (4, 0));
        assert_eq!(m.get(0x12_0001), Some((3, 1)));
        assert_eq!(m.leaves(), 2);
        m.clear_range(0x12_0001, 0x12_0101);
        assert_eq!(
            (m.get(0x12_0000), m.get(0x12_0001), m.get(0x12_0100)),
            (Some((3, 0)), None, None)
        );
        m.set(PCMAP_SPAN, (1, 1));
        assert_eq!(m.get(PCMAP_SPAN), None);
        m.clear_range(5, 5);
        m.clear_range(PCMAP_SPAN - 1, u32::MAX);
        assert_eq!(PCMAP_SPAN, 0x20_0000);
    }

    #[test]
    fn install_enters_every_uop_boundary_and_drop_block_takes_them_out_again() {
        use crate::core::support::core_with;
        use crate::core::uop::lower::{builder::BlockBuilder, decode::decode_at};
        let mut c = core_with(&[0x0001, 0x0001, 0x0001]);
        let (pc, mut b) = (c.pc, BlockBuilder::new(c.pc, false, false));
        let mut p = pc;
        for _ in 0..3 {
            let i = decode_at(&mut c, p);
            b.interp(&i);
            p = i.next_pc;
        }
        let (block, uops) = b.finish(Default::default());
        let mut h = UopHot::new();
        assert_eq!(h.install(vec![block.clone()], uops.clone()), (1, 3));
        assert_eq!(
            h.install(vec![block], uops),
            (1, 3),
            "a second copy gets arena offset 3"
        );
        assert_eq!((h.blocks[1].first, h.arena.len()), (3, 6));
        assert_eq!(
            h.pcmap(false).get(pc + 1),
            Some((1, 1)),
            "the later install wins"
        );
        let page = CodePages::page_of(pc).unwrap();
        assert_eq!(h.page_blocks[page], [0, 1]);
        assert!(
            h.dropped.capacity() >= h.blocks.len(),
            "dropping a block never allocates"
        );
        assert!(h.drop_block(1) && !h.drop_block(1) && !h.drop_block(99));
        assert_eq!(h.pcmap(false).get(pc), None);
        assert_eq!(
            (h.epoch, h.dropped.clone(), h.blocks[1].len),
            (1, vec![(pc, pc + 2 + 3)], 0)
        );
    }

    #[test]
    fn a_block_is_entered_through_the_pc_map_of_its_own_simd_mode_only() {
        use crate::core::support::core_with;
        use crate::core::uop::lower::{builder::BlockBuilder, decode::decode_at};
        let mut c = core_with(&[0x0001, 0x0001]);
        let pc = c.pc;
        let insn = decode_at(&mut c, pc);
        let mut h = UopHot::new();
        for simd in [false, true] {
            let mut b = BlockBuilder::new(pc, simd, false);
            b.interp(&insn);
            let (block, uops) = b.finish(Default::default());
            h.install(vec![block], uops);
        }
        assert_eq!(
            (h.pcmap(false).get(pc), h.pcmap(true).get(pc)),
            (Some((0, 0)), Some((1, 0)))
        );
        assert!(h.drop_block(1));
        assert_eq!(
            (h.pcmap(false).get(pc), h.pcmap(true).get(pc)),
            (Some((0, 0)), None),
            "dropping the PEy-on variant leaves the PEy-off one"
        );
    }
}
