//! Memory access paths of the move handlers.
//!
//! OWNER: W2-MOVE. The fast path is only sound where no peripheral claims an address, so the
//! SRAM windows must stay disjoint from `periph_claims` (ASSUMED until the package's
//! `periph_claims_disjoint_from_sram_windows` pins it; the core's peripheral range starts at
//! `0x2000_0000`, read in `periph/glue.rs`). The skeleton exposes the window test the package
//! builds on: the SRAM normal-word window and the short-word alias window of `memory.rs` (imported,
//! not copied). Everything else goes through `Core::dm_read` / `Core::dm_write`.
use crate::core::memory::{SHORT_WORDS, SRAM_WORDS};

/// Whether normal-word data address `addr` is backed by the internal SRAM.
#[inline(always)]
pub(crate) fn in_sram_window(addr: u32) -> bool {
    SRAM_WORDS.contains(&addr)
}

/// Whether `addr` is in the short-word alias window of the SRAM (writes there reach SRAM through
/// `dm_write`, which calls the invalidation hook, so handlers never bypass it).
#[inline(always)]
pub(crate) fn in_short_window(addr: u32) -> bool {
    SHORT_WORDS.contains(&addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_are_the_ones_of_memory_rs_and_do_not_overlap() {
        assert!(in_sram_window(0x9_0000) && in_sram_window(0xF_FFFF) && !in_sram_window(0x10_0000));
        assert!(
            in_short_window(0x10_0000) && in_short_window(0x1F_FFFF) && !in_short_window(0x20_0000)
        );
        assert!(!in_sram_window(0x8_FFFF) && !in_short_window(0xF_FFFF));
        assert!(
            !in_sram_window(0x2000_0000),
            "the peripheral range is outside both windows"
        );
        const _: () = assert!(SRAM_WORDS.end <= SHORT_WORDS.start);
    }
}
