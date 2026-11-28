//! OWNER: W1-NCORE. Raw views of guest memory for the native tier (feature `sb-native`).
//!
//! Spec (design note 9, WP W1-NCORE): [`Mem::raw_parts`] returns the page table and the arena as
//! raw pointers, which is safe to do (nothing is dereferenced here). The native code reads
//! `table[addr >> 12]`; an entry is `flags | arena page index`, index 0 meaning unmapped, and the
//! host address of a mapped byte is `arena - 4096 + (index << 12) + (addr & 0xFFF)`. Only Rust
//! code can map a page, which can move the arena, so the pointers are valid only until the next
//! `&mut Mem` use: the driver calls `raw_parts` before every native entry and stores the result
//! in `NativeCtl`. Native code never runs while another borrow of the `Mem` is live.

use super::Mem;

/// Pointers into a [`Mem`]: the page table (`1 << 20` entries) and the page arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawMem {
    pub table: *const u32,
    pub arena: *mut u8,
}

impl Mem {
    /// Raw views of the table and the arena, valid until the next mutable use of `self`.
    pub fn raw_parts(&mut self) -> RawMem {
        RawMem {
            table: self.table.as_ptr(),
            arena: self.arena.as_mut_ptr(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointers_are_those_of_the_table_and_the_arena() {
        let mut m = Mem::new();
        m.write32(0x8000_0000, 1);
        let raw = m.raw_parts();
        assert_eq!(raw.table, m.table.as_ptr());
        assert_eq!(raw.arena, m.arena.as_mut_ptr());
        assert!(!raw.table.is_null() && !raw.arena.is_null());
    }

    #[test]
    fn the_arena_pointer_follows_a_reallocation_so_it_must_be_refreshed_before_every_entry() {
        let mut m = Mem::new();
        m.write32(0x8000_0000, 1);
        let before = m.raw_parts();
        for page in 1..200u32 {
            m.write32(0x8000_0000 + page * 0x1000, page); // grows the arena
        }
        let after = m.raw_parts();
        assert_eq!(after.table, before.table, "the table never moves");
        assert_eq!(after.arena, m.arena.as_mut_ptr());
        assert!(m.arena_bytes() >= 200 * 0x1000);
    }

    #[test]
    fn a_table_entry_is_the_arena_page_index_and_the_flags() {
        let mut m = Mem::new();
        m.write32(0x8000_0000, 0xAABB_CCDD);
        let idx = m.table[0x80000] & 0x0FFF_FFFF;
        assert_eq!(idx, 1, "the first mapped page has index 1");
        assert_eq!(
            &m.arena[(idx as usize - 1) * 4096..][..4],
            &0xAABB_CCDDu32.to_le_bytes()
        );
    }
}
