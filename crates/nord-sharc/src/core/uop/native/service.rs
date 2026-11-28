//! Native translation service: the worker side that emits, validates and hands back native code.
//!
//! OWNER: W4-NSVC. Requests arrive through `service::Service`; the worker emits a block, runs
//! `tv::validate` on it and only a validated block is returned for installation into the
//! `NativeTable` at a pass boundary (a pointer write; a full table drops the install and returns
//! the block to the worker). The request ring never allocates on the audio side. Skeleton: the
//! install decision, which is what the worker applies to every finished block.
use super::cache::NativeTable;
use super::tv;

/// What the audio side does with a finished, validated-or-not native block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// Stored in this slot.
    Installed(u32),
    /// The block failed validation; it stays in Tier 1.
    Rejected,
    /// The table is full; the block goes back to the worker.
    TableFull,
}

/// Installs `entry` into `table` if `mismatches` validation failures allow it.
pub fn install(table: &mut NativeTable, entry: usize, mismatches: u32) -> Install {
    if !tv::accepts(mismatches) {
        return Install::Rejected;
    }
    table
        .install(entry)
        .map_or(Install::TableFull, Install::Installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::native::cache::SLOTS;

    #[test]
    fn a_validated_block_installs_a_failed_one_does_not_and_a_full_table_returns_it() {
        let mut t = NativeTable::new();
        assert_eq!(install(&mut t, 0x1000, 0), Install::Installed(0));
        assert_eq!(install(&mut t, 0x2000, 2), Install::Rejected);
        assert_eq!(t.len(), 1);
        for k in 1..SLOTS {
            install(&mut t, k + 0x10, 0);
        }
        assert_eq!(install(&mut t, 0xABCD, 0), Install::TableFull);
    }
}
