//! The native code table: fixed-capacity slots holding block entry addresses.
//!
//! OWNER: W4-NCORE. Installation happens at a pass boundary into a pre-sized table (a pointer
//! write, no allocation); a full table refuses the install; a dropped block's slot becomes a
//! tombstone that routes to Tier 1, and code memory is freed at pass boundaries only (design
//! 7.4 rule 4). Skeleton: the table is complete; the code memory behind the addresses is
//! W4-NCORE.
use crate::core::uop::types::NONE;

/// Slots in the table. ASSUMED: ORG Core2 has about 1,000 blocks (design section 5); 4,096 leaves
/// room for the other cores' programs and overlays.
pub const SLOTS: usize = 4096;

/// Entry address per slot; 0 marks a free or tombstoned slot.
pub struct NativeTable {
    entries: Box<[usize]>,
    used: usize,
}

impl Default for NativeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeTable {
    /// An empty table of [`SLOTS`] slots.
    pub fn new() -> Self {
        NativeTable {
            entries: vec![0; SLOTS].into_boxed_slice(),
            used: 0,
        }
    }

    /// Slots in the table.
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Slots holding an entry.
    pub fn len(&self) -> usize {
        self.used
    }

    /// Whether no slot holds an entry.
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// Stores `entry` (non-zero) in the first free slot and returns its index, or `None` when the
    /// table is full. Never allocates.
    pub fn install(&mut self, entry: usize) -> Option<u32> {
        debug_assert!(entry != 0, "0 marks a free slot");
        let slot = self.entries.iter().position(|e| *e == 0)?;
        self.entries[slot] = entry;
        self.used += 1;
        u32::try_from(slot).ok().filter(|s| *s != NONE)
    }

    /// The entry of `slot`, or `None` when it is free, tombstoned or out of range.
    pub fn get(&self, slot: u32) -> Option<usize> {
        self.entries.get(slot as usize).copied().filter(|e| *e != 0)
    }

    /// Frees `slot`; its block runs in Tier 1 from now on.
    pub fn tombstone(&mut self, slot: u32) {
        if let Some(e) = self.entries.get_mut(slot as usize) {
            self.used -= usize::from(*e != 0);
            *e = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_get_and_tombstone_reuse_slots_without_growing() {
        let mut t = NativeTable::new();
        assert_eq!((t.capacity(), t.is_empty()), (SLOTS, true));
        assert_eq!((t.install(0x1000), t.install(0x2000)), (Some(0), Some(1)));
        assert_eq!((t.get(1), t.get(2), t.len()), (Some(0x2000), None, 2));
        t.tombstone(0);
        t.tombstone(0);
        assert_eq!((t.get(0), t.len()), (None, 1));
        assert_eq!(t.install(0x3000), Some(0), "the freed slot is reused first");
        assert_eq!(t.capacity(), SLOTS);
    }

    #[test]
    fn a_full_table_refuses_the_install() {
        let mut t = NativeTable::new();
        for k in 0..SLOTS {
            assert!(t.install(k + 1).is_some());
        }
        assert_eq!(t.install(0xFFFF), None);
        assert_eq!(t.get(SLOTS as u32), None);
    }
}
