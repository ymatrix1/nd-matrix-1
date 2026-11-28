//! Run state that lives inside [`crate::core::Core`] by value: counters, flags, the code-page
//! bitmap and the dirty set. It is separate from `UopHot` because the invalidation hook runs
//! while `UopHot` is taken out of the core (`uop_try` does `self.uop.take()`), so the hook must
//! reach its state through `Core` alone and without any heap.
//!
//! OWNER: W0 (skeleton); contract changes go back to W0.
use super::config::{UopConfig, UopStats};
use super::types::{NONE, PAGE_SHIFT};
use crate::core::memory::SHORT_WORDS;
use crate::core::Fault;

/// `u64` words of the code-page bitmap: 64 x 64 = 4096 pages of 256 short words.
pub const CODE_PAGE_WORDS: usize = 64;
/// Pages the bitmap covers.
pub const CODE_PAGES: usize = CODE_PAGE_WORDS * 64;
/// First short-word pc of the bitmap: the start of the SRAM short-word window (`memory.rs`,
/// `SHORT_WORDS`), where VISA code lives (`0x12_0000..` in the firmware images).
pub const CODE_BASE: u32 = SHORT_WORDS.start;
/// Short words the bitmap covers (1 Mi, the whole `SHORT_WORDS` window).
pub const CODE_SPAN: u32 = (CODE_PAGES as u32) << PAGE_SHIFT;

/// One bit per code page, over the short-word window. ISA-mode blocks (pc below the window, the
/// 48-bit word space of the boot kernels) have no pages: their code reaches SRAM through
/// `isa_resync`, which calls `invalidate_code_cache` and so `uop_note_all`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodePages {
    pub bits: [u64; CODE_PAGE_WORDS],
}

impl Default for CodePages {
    fn default() -> Self {
        Self::new()
    }
}

impl CodePages {
    /// An empty bitmap.
    pub const fn new() -> Self {
        CodePages {
            bits: [0; CODE_PAGE_WORDS],
        }
    }

    /// Whether no page is marked.
    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|w| *w == 0)
    }

    /// Unmarks every page.
    pub fn clear(&mut self) {
        self.bits = [0; CODE_PAGE_WORDS];
    }

    /// The page of short-word pc `pc`, `None` outside the window.
    pub const fn page_of(pc: u32) -> Option<usize> {
        let o = pc.wrapping_sub(CODE_BASE);
        if o < CODE_SPAN {
            Some((o >> PAGE_SHIFT) as usize)
        } else {
            None
        }
    }

    /// Whether `page` is marked.
    pub fn test(&self, page: usize) -> bool {
        page < CODE_PAGES && self.bits[page >> 6] >> (page & 63) & 1 != 0
    }

    /// Marks every page of the short words `lo..hi` (clamped to the window). `hi` must be past
    /// the last parcel of the last instruction, so a write to any parcel of translated code
    /// lands on a marked page without a safety margin.
    pub fn set_range(&mut self, lo: u32, hi: u32) {
        let (lo, hi) = (lo.max(CODE_BASE), hi.min(CODE_BASE + CODE_SPAN));
        if lo >= hi {
            return;
        }
        let (p0, p1) = (
            ((lo - CODE_BASE) >> PAGE_SHIFT) as usize,
            ((hi - 1 - CODE_BASE) >> PAGE_SHIFT) as usize,
        );
        for p in p0..=p1 {
            self.bits[p >> 6] |= 1 << (p & 63);
        }
    }

    /// Bit `k` is set when a marked page of bitmap word `k` (pages `64k..64k+64`) overlaps the
    /// short words `first_short..first_short + count`; 0 means no hit.
    pub fn hit_mask(&self, first_short: u32, count: u32) -> u64 {
        let lo = first_short.max(CODE_BASE);
        let hi = first_short.saturating_add(count).min(CODE_BASE + CODE_SPAN);
        if lo >= hi {
            return 0;
        }
        let (p0, p1) = (
            ((lo - CODE_BASE) >> PAGE_SHIFT) as usize,
            ((hi - 1 - CODE_BASE) >> PAGE_SHIFT) as usize,
        );
        (p0..=p1)
            .filter(|&p| self.test(p))
            .fold(0, |m, p| m | 1 << (p >> 6))
    }

    /// Whether any marked page overlaps `first_short..first_short + count`.
    pub fn hit(&self, first_short: u32, count: u32) -> bool {
        self.hit_mask(first_short, count) != 0
    }
}

/// `UopRun::flags`: a store hit a code page since the last revalidation.
pub const RF_SMC: u32 = 1;
/// `UopRun::flags`: shadow checking is on (store handlers log their writes).
pub const RF_SHADOW: u32 = 2;
/// `UopRun::flags`: tracing is on.
pub const RF_TRACE: u32 = 4;

/// Everything of the engine that must be reachable from `Core` without the boxed `UopHot`.
#[derive(Clone, Debug)]
pub struct UopRun {
    pub cfg: UopConfig,
    pub stats: UopStats,
    pub code_pages: CodePages,
    /// `RF_*` bits, checked once after every store handler.
    pub flags: u32,
    /// A fault an `Interp` uop raised, left for `uop_take_fault`.
    pub fault: Option<Fault>,
    /// pc at which block chaining returns to the caller (the pass idle pc); [`NONE`] = none.
    pub stop_pc: u32,
    /// Bit `k` is set when a write hit a code page of `code_pages.bits[k]` (64 pages, 16 Ki short
    /// words) since the last revalidation; `u64::MAX` after a full code-cache flush. Set by the
    /// write hook, consumed by revalidation and `uop_maintain`.
    pub dirty: u64,
    /// The engine is armed (`enable_uop` marked at least one code page): the write hook does
    /// work only while this is set, so a disabled engine costs one byte load per SRAM store.
    pub active: bool,
}

impl Default for UopRun {
    fn default() -> Self {
        Self::new()
    }
}

impl UopRun {
    /// The inert state of a core without the engine.
    pub fn new() -> Self {
        UopRun {
            cfg: UopConfig::default(),
            stats: UopStats::default(),
            code_pages: CodePages::new(),
            flags: 0,
            fault: None,
            stop_pc: NONE,
            dirty: 0,
            active: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_pages_cover_short_window() {
        // the bitmap spans exactly the SRAM short-word window of memory.rs (1 Mi short words)
        assert_eq!(CODE_BASE, SHORT_WORDS.start);
        assert_eq!(CODE_BASE + CODE_SPAN, SHORT_WORDS.end);
        assert_eq!(CODE_SPAN as usize, SHORT_WORDS.len());
        assert_eq!(CODE_PAGES << PAGE_SHIFT, 1 << 20);
        assert_eq!(std::mem::size_of::<CodePages>(), 512);
        // first and last short word map to the first and last page, the words around do not
        assert_eq!(CodePages::page_of(CODE_BASE), Some(0));
        assert_eq!(
            CodePages::page_of(CODE_BASE + CODE_SPAN - 1),
            Some(CODE_PAGES - 1)
        );
        assert_eq!(CodePages::page_of(CODE_BASE - 1), None);
        assert_eq!(CodePages::page_of(CODE_BASE + CODE_SPAN), None);
        // the firmware code window starts inside it
        assert!(CodePages::page_of(0x12_0000).is_some());
    }

    #[test]
    fn set_range_marks_pages_and_hit_sees_only_marked_ones() {
        let mut p = CodePages::new();
        assert!(p.is_empty());
        // one parcel past a page boundary marks two pages
        p.set_range(0x12_00F0, 0x12_0101);
        assert!(p.test(0x200) && p.test(0x201) && !p.test(0x202) && !p.test(0x1FF));
        assert!(p.hit(0x12_0000, 1) && p.hit(0x12_0100, 1) && p.hit(0x12_0000, 0x400));
        assert!(!p.hit(0x12_0200, 1) && !p.hit(0x11_FFFF, 1));
        assert_eq!(p.hit_mask(0x12_0000, 1), 1 << 8, "page 0x200 is in word 8");
        p.set_range(0x13_0000, 0x13_0001);
        assert_eq!(
            p.hit_mask(0x13_0000, 1),
            1 << ((0x13_0000 - CODE_BASE) >> 14)
        );
        p.clear();
        assert!(p.is_empty() && !p.hit(0x12_0000, 0x400));
    }

    #[test]
    fn ranges_outside_the_window_are_clamped_or_ignored() {
        let mut p = CodePages::new();
        p.set_range(0, 0x10);
        p.set_range(CODE_BASE + CODE_SPAN, CODE_BASE + CODE_SPAN + 5);
        p.set_range(5, 5);
        assert!(p.is_empty());
        p.set_range(0, CODE_BASE + 1);
        assert!(p.test(0) && !p.test(1));
        assert!(!p.hit(0, 0x10) && !p.hit(u32::MAX - 2, 8));
    }

    #[test]
    fn a_new_run_state_is_inert() {
        let r = UopRun::new();
        assert!(!r.active && r.code_pages.is_empty());
        assert_eq!((r.flags, r.dirty, r.stop_pc), (0, 0, NONE));
        assert!(r.fault.is_none());
        assert_eq!((RF_SMC, RF_SHADOW, RF_TRACE), (1, 2, 4));
    }
}
