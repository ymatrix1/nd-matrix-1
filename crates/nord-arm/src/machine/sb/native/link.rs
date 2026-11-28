//! OWNER: W3-NINT. Link slots of native traces: the patchable successor of an exit.
//!
//! Spec (design note 9, WP W3-NINT): chaining v1 returns to the driver at every exit (about 20
//! cycles); v2 (gated on the spike) gives every `Direct` exit a slot that holds the successor's
//! code address once it is known, and the exit becomes `return_call_indirect` through the slot
//! with the Tail convention. A slot is cleared when its target trace is released, so no native
//! code ever jumps into freed memory. The skeleton is the slot table only.

use crate::machine::sb::uop::{SbId, NO_SB};

/// One patchable successor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkSlot {
    /// Code address of the successor, 0 while unlinked.
    pub code: usize,
    /// The successor's trace id (`NO_SB` while unlinked), to clear the slot on release.
    pub id: SbId,
}

impl LinkSlot {
    pub const UNLINKED: LinkSlot = LinkSlot { code: 0, id: NO_SB };
}

/// All slots of a backend. An index is stable for the life of the table.
#[derive(Default, Debug)]
pub struct LinkTable {
    slots: Vec<LinkSlot>,
}

impl LinkTable {
    /// Allocate `n` unlinked slots; returns the index of the first.
    pub fn alloc(&mut self, n: usize) -> usize {
        let base = self.slots.len();
        self.slots.resize(base + n, LinkSlot::UNLINKED);
        base
    }

    pub fn get(&self, i: usize) -> LinkSlot {
        self.slots[i]
    }

    /// Point slot `i` at the code of trace `id`.
    pub fn link(&mut self, i: usize, code: usize, id: SbId) {
        self.slots[i] = LinkSlot { code, id };
    }

    /// Unlink every slot that points at trace `id` (its code is going away); returns how many.
    pub fn unlink_target(&mut self, id: SbId) -> usize {
        let mut n = 0;
        for s in self.slots.iter_mut().filter(|s| s.id == id) {
            *s = LinkSlot::UNLINKED;
            n += 1;
        }
        n
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_start_unlinked_and_are_allocated_in_runs() {
        let mut t = LinkTable::default();
        assert!(t.is_empty());
        assert_eq!((t.alloc(2), t.alloc(3)), (0, 2));
        assert_eq!(t.len(), 5);
        assert_eq!(t.get(4), LinkSlot::UNLINKED);
    }

    #[test]
    fn releasing_a_target_unlinks_every_slot_that_pointed_at_it() {
        let mut t = LinkTable::default();
        t.alloc(4);
        t.link(0, 0x1000, 7);
        t.link(2, 0x1000, 7);
        t.link(3, 0x2000, 8);
        assert_eq!(t.unlink_target(7), 2);
        assert_eq!([t.get(0), t.get(1), t.get(2)], [LinkSlot::UNLINKED; 3]);
        assert_eq!(
            t.get(3),
            LinkSlot {
                code: 0x2000,
                id: 8
            }
        );
        assert_eq!(t.unlink_target(7), 0);
    }
}
