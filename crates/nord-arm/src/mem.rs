//! Sparse paged guest memory with per-page hook flags.
//!
//! The 4 GiB guest space is a table of `1 << 20` page entries. An entry is `0` (unmapped) or the
//! index of a 4 KiB page in one flat arena, plus flag bits that the bus checks on every access:
//! [`HOOK_R`], [`HOOK_W`] and [`HOOK_X`]. Unmapped accesses map a zero page on demand, which is
//! what the Python/Unicorn harness does (`ArmEmu._on_unmapped`).
//!
//! ASSUMED: all pages are readable, writable and executable (the harness never sets permissions).

#[cfg(feature = "sb-native")]
pub mod raw;

pub const PAGE_SHIFT: u32 = 12;
pub const PAGE_SIZE: usize = 1 << PAGE_SHIFT;
const PAGES: usize = 1 << (32 - PAGE_SHIFT);

/// Page flag: reads go through the device hook.
pub const HOOK_R: u32 = 1 << 31;
/// Page flag: writes go through the device hook.
pub const HOOK_W: u32 = 1 << 30;
/// Page flag: an instruction fetch on this page may need a code hook (stub, null guard).
pub const HOOK_X: u32 = 1 << 29;
/// Page flag: the predecode cache holds decoded instructions of this page; a write to it (or a
/// new `HOOK_X`) queues the page in [`Mem::stale_code`] and clears the flag.
pub const HOOK_C: u32 = 1 << 28;
const FLAGS: u32 = HOOK_R | HOOK_W | HOOK_X | HOOK_C;

/// Sparse guest memory.
#[derive(Clone)]
pub struct Mem {
    table: Vec<u32>,
    arena: Vec<u8>,
    /// Pages mapped on demand by an unmapped access (page base addresses, in order).
    pub auto_mapped: Vec<u32>,
    /// Every on-demand mapping, also those past `AUTO_MAPPED_CAP`.
    pub auto_mapped_count: u64,
    /// Page numbers whose decoded code went stale (written, or hooked) since the predecode cache
    /// last drained them; the run loop checks this before each fetch.
    pub stale_code: Vec<u32>,
}

/// Entries kept in `auto_mapped`.
pub const AUTO_MAPPED_CAP: usize = 4096;

impl Default for Mem {
    fn default() -> Self {
        Self::new()
    }
}

impl Mem {
    pub fn new() -> Self {
        Self {
            table: vec![0; PAGES],
            arena: Vec::new(),
            auto_mapped: Vec::new(),
            auto_mapped_count: 0,
            stale_code: Vec::new(),
        }
    }

    /// Bytes of mapped guest memory (the arena size).
    pub fn arena_bytes(&self) -> usize {
        self.arena.len()
    }

    /// Reserve arena room for `pages` more pages so on-demand mapping does not reallocate (and
    /// copy) the arena during a timed run.
    pub fn reserve_pages(&mut self, pages: usize) {
        self.arena.reserve(pages * PAGE_SIZE);
    }

    /// Page flags of the page holding `addr` (`HOOK_*` bits).
    #[inline(always)]
    pub fn flags(&self, addr: u32) -> u32 {
        self.table[(addr >> PAGE_SHIFT) as usize] & FLAGS
    }

    /// Set `flag` on every page of `[begin, end]`. Pages stay unmapped (a flagged page is mapped
    /// on demand by its first access, exactly as an unflagged one).
    pub fn add_flag(&mut self, begin: u32, end: u32, flag: u32) {
        for page in (begin >> PAGE_SHIFT)..=(end >> PAGE_SHIFT) {
            if flag & HOOK_X != 0 {
                self.mark_stale(page);
            }
            self.table[page as usize] |= flag;
        }
    }

    /// Queue `page` for decode invalidation when it holds decoded code.
    fn mark_stale(&mut self, page: u32) {
        if self.table[page as usize] & HOOK_C != 0 {
            self.table[page as usize] &= !HOOK_C;
            self.stale_code.push(page);
        }
    }

    /// Flag `addr`'s page as holding decoded code (the page must be mapped).
    pub fn mark_code(&mut self, addr: u32) {
        self.table[(addr >> PAGE_SHIFT) as usize] |= HOOK_C;
    }

    fn ensure(&mut self, page: u32) {
        if self.table[page as usize] & !FLAGS == 0 {
            let idx = (self.arena.len() / PAGE_SIZE) as u32 + 1;
            self.arena.resize(self.arena.len() + PAGE_SIZE, 0);
            self.table[page as usize] = (self.table[page as usize] & FLAGS) | idx;
        }
    }

    /// Map zero pages over `[base, base + size)` (already mapped pages keep their content).
    pub fn map(&mut self, base: u32, size: u32) {
        if size == 0 {
            return;
        }
        let last = base.saturating_add(size - 1) >> PAGE_SHIFT;
        for page in (base >> PAGE_SHIFT)..=last {
            self.ensure(page);
        }
    }

    pub fn is_mapped(&self, addr: u32) -> bool {
        self.table[(addr >> PAGE_SHIFT) as usize] & !FLAGS != 0
    }

    /// Arena offset of `addr`, mapping a zero page when the address was unmapped.
    #[inline(always)]
    fn off(&mut self, addr: u32) -> usize {
        let e = self.table[(addr >> PAGE_SHIFT) as usize] & !FLAGS;
        if e == 0 {
            return self.map_on_demand(addr);
        }
        (e as usize - 1) * PAGE_SIZE + (addr as usize & (PAGE_SIZE - 1))
    }

    /// [`Mem::off`] for a store: a page with decoded code goes stale first.
    #[inline(always)]
    fn off_w(&mut self, addr: u32) -> usize {
        let e = self.table[(addr >> PAGE_SHIFT) as usize];
        let idx = (e & !FLAGS) as usize;
        if idx == 0 || e & HOOK_C != 0 {
            return self.off_w_slow(addr);
        }
        (idx - 1) * PAGE_SIZE + (addr as usize & (PAGE_SIZE - 1))
    }

    #[cold]
    fn off_w_slow(&mut self, addr: u32) -> usize {
        self.mark_stale(addr >> PAGE_SHIFT);
        self.off(addr)
    }

    #[cold]
    fn map_on_demand(&mut self, addr: u32) -> usize {
        let page = addr >> PAGE_SHIFT;
        self.ensure(page);
        self.auto_mapped_count += 1;
        if self.auto_mapped.len() < AUTO_MAPPED_CAP {
            self.auto_mapped.push(page << PAGE_SHIFT);
        }
        let e = self.table[page as usize] & !FLAGS;
        (e as usize - 1) * PAGE_SIZE + (addr as usize & (PAGE_SIZE - 1))
    }

    #[inline(always)]
    pub fn read8(&mut self, addr: u32) -> u8 {
        let o = self.off(addr);
        self.arena[o]
    }

    #[inline(always)]
    pub fn write8(&mut self, addr: u32, v: u8) {
        let o = self.off_w(addr);
        self.arena[o] = v;
    }

    #[inline(always)]
    pub fn read16(&mut self, addr: u32) -> u16 {
        if addr & 0xFFF <= 0xFFE {
            let o = self.off(addr);
            u16::from_le_bytes(self.arena[o..o + 2].try_into().unwrap())
        } else {
            u16::from_le_bytes([self.read8(addr), self.read8(addr.wrapping_add(1))])
        }
    }

    #[inline(always)]
    pub fn write16(&mut self, addr: u32, v: u16) {
        let b = v.to_le_bytes();
        if addr & 0xFFF <= 0xFFE {
            let o = self.off_w(addr);
            self.arena[o..o + 2].copy_from_slice(&b);
        } else {
            self.write8(addr, b[0]);
            self.write8(addr.wrapping_add(1), b[1]);
        }
    }

    #[inline(always)]
    pub fn read32(&mut self, addr: u32) -> u32 {
        if addr & 0xFFF <= 0xFFC {
            let o = self.off(addr);
            u32::from_le_bytes(self.arena[o..o + 4].try_into().unwrap())
        } else {
            let mut b = [0u8; 4];
            for (i, x) in b.iter_mut().enumerate() {
                *x = self.read8(addr.wrapping_add(i as u32));
            }
            u32::from_le_bytes(b)
        }
    }

    #[inline(always)]
    pub fn write32(&mut self, addr: u32, v: u32) {
        let b = v.to_le_bytes();
        if addr & 0xFFF <= 0xFFC {
            let o = self.off_w(addr);
            self.arena[o..o + 4].copy_from_slice(&b);
        } else {
            for (i, x) in b.iter().enumerate() {
                self.write8(addr.wrapping_add(i as u32), *x);
            }
        }
    }

    /// Table entry of `addr`'s page (page index plus flags), for the `*_e` accessors: a bus that
    /// already looked at the flags passes the entry on instead of looking the page up again.
    #[inline(always)]
    pub fn entry(&self, addr: u32) -> u32 {
        self.table[(addr >> PAGE_SHIFT) as usize]
    }

    /// Arena offset of `addr` when `e` (from [`Mem::entry`]) maps it, `addr..addr + len` stays in
    /// the page and, for a store, no decoded code lives there; `None` sends the caller to the
    /// general accessor.
    #[inline(always)]
    fn fast(e: u32, addr: u32, len: u32, store: bool) -> Option<usize> {
        let idx = (e & !FLAGS) as usize;
        if idx == 0 || addr & 0xFFF > 0x1000 - len || (store && e & HOOK_C != 0) {
            return None;
        }
        Some((idx - 1) * PAGE_SIZE + (addr as usize & (PAGE_SIZE - 1)))
    }

    #[inline(always)]
    pub fn read8_e(&mut self, e: u32, addr: u32) -> u8 {
        match Self::fast(e, addr, 1, false) {
            Some(o) => self.arena[o],
            None => self.read8(addr),
        }
    }

    #[inline(always)]
    pub fn read16_e(&mut self, e: u32, addr: u32) -> u16 {
        match Self::fast(e, addr, 2, false) {
            Some(o) => u16::from_le_bytes(self.arena[o..o + 2].try_into().unwrap()),
            None => self.read16(addr),
        }
    }

    #[inline(always)]
    pub fn read32_e(&mut self, e: u32, addr: u32) -> u32 {
        match Self::fast(e, addr, 4, false) {
            Some(o) => u32::from_le_bytes(self.arena[o..o + 4].try_into().unwrap()),
            None => self.read32(addr),
        }
    }

    #[inline(always)]
    pub fn write8_e(&mut self, e: u32, addr: u32, v: u8) {
        match Self::fast(e, addr, 1, true) {
            Some(o) => self.arena[o] = v,
            None => self.write8(addr, v),
        }
    }

    #[inline(always)]
    pub fn write16_e(&mut self, e: u32, addr: u32, v: u16) {
        match Self::fast(e, addr, 2, true) {
            Some(o) => self.arena[o..o + 2].copy_from_slice(&v.to_le_bytes()),
            None => self.write16(addr, v),
        }
    }

    #[inline(always)]
    pub fn write32_e(&mut self, e: u32, addr: u32, v: u32) {
        match Self::fast(e, addr, 4, true) {
            Some(o) => self.arena[o..o + 4].copy_from_slice(&v.to_le_bytes()),
            None => self.write32(addr, v),
        }
    }

    /// Copy `data` to guest memory at `addr` (maps the pages).
    pub fn write_bytes(&mut self, addr: u32, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.write8(addr.wrapping_add(i as u32), *b);
        }
    }

    /// Read `len` bytes from guest memory.
    pub fn read_bytes(&mut self, addr: u32, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| self.read8(addr.wrapping_add(i as u32)))
            .collect()
    }

    /// Zero `len` bytes at `addr` (maps the pages like a store would).
    pub fn zero(&mut self, addr: u32, len: usize) {
        let mut done = 0usize;
        while done < len {
            let a = addr.wrapping_add(done as u32);
            let n = (PAGE_SIZE - (a as usize & (PAGE_SIZE - 1))).min(len - done);
            let o = self.off_w(a);
            self.arena[o..o + n].fill(0);
            done += n;
        }
    }

    /// FNV-style hash of every mapped page (address and content) for state comparison.
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xCBF2_9CE4_8422_2325;
        for p in 0..PAGES {
            let e = self.table[p] & !FLAGS;
            if e == 0 {
                continue;
            }
            let o = (e as usize - 1) * PAGE_SIZE;
            let page = &self.arena[o..o + PAGE_SIZE];
            if page.iter().all(|b| *b == 0) {
                continue; // zero pages hash like unmapped ones: mapping is not state
            }
            h = (h ^ p as u64).wrapping_mul(0x100_0000_01B3);
            for c in page.as_chunks::<8>().0 {
                h = (h ^ u64::from_le_bytes(*c)).wrapping_mul(0x100_0000_01B3);
                h ^= h >> 29;
            }
        }
        h
    }

    /// Mapped page base addresses in ascending order (for state hashing and snapshots).
    pub fn mapped_pages(&self) -> Vec<u32> {
        (0..PAGES as u32)
            .filter(|p| self.table[*p as usize] & !FLAGS != 0)
            .map(|p| p << PAGE_SHIFT)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_clears_across_pages_maps_them_and_stales_decoded_code() {
        let mut m = Mem::new();
        m.write32(0x1FFC, 0xFFFF_FFFF);
        m.write32(0x2000, 0xFFFF_FFFF);
        m.mark_code(0x2000);
        m.zero(0x1FFE, 6); // 2 bytes of page 1, 4 of page 2
        assert_eq!((m.read32(0x1FFC), m.read32(0x2000)), (0x0000_FFFF, 0));
        assert_eq!(m.stale_code, vec![2]);
        m.zero(0x9000, 5000); // unmapped: maps two pages
        assert!(m.is_mapped(0x9000) && m.is_mapped(0xA000));
    }

    #[test]
    fn a_store_to_a_page_with_decoded_code_queues_it_once_and_clears_the_flag() {
        let mut m = Mem::new();
        m.write32(0x5000, 1);
        m.mark_code(0x5000);
        assert_eq!(m.flags(0x5000) & HOOK_C, HOOK_C);
        m.write8(0x5FFF, 9); // other byte of the same page
        assert_eq!(m.stale_code, vec![5]);
        assert_eq!(m.flags(0x5000) & HOOK_C, 0);
        m.write32(0x5004, 2); // flag already clear: nothing queued
        assert_eq!(m.stale_code, vec![5]);
        assert_eq!(m.read32(0x5000), 1); // content untouched by the flag handling
    }

    #[test]
    fn reads_keep_decoded_code_and_hooking_a_page_makes_it_stale() {
        let mut m = Mem::new();
        m.write32(0x6000, 7);
        m.mark_code(0x6000);
        assert_eq!(m.read32(0x6000), 7);
        assert!(m.stale_code.is_empty());
        m.add_flag(0x6000, 0x6000, HOOK_X);
        assert_eq!(m.stale_code, vec![6]);
        assert!(m.is_mapped(0x6000)); // the flag bits never leak into the page index
    }

    #[test]
    fn unmapped_read_maps_zero_page_and_logs_it() {
        let mut m = Mem::new();
        assert_eq!(m.read32(0x1234_5678), 0);
        assert!(m.is_mapped(0x1234_5000));
        assert_eq!(m.auto_mapped, vec![0x1234_5000]);
    }

    #[test]
    fn map_of_zero_bytes_maps_nothing_and_a_range_to_the_top_does_not_wrap() {
        let mut m = Mem::new();
        m.map(0x1000, 0);
        assert_eq!(m.arena_bytes(), 0);
        m.map(0xFFFF_F000, 0x2000);
        assert_eq!(m.arena_bytes(), PAGE_SIZE);
    }

    #[test]
    fn little_endian_round_trip_and_page_crossing() {
        let mut m = Mem::new();
        m.write32(0x1000, 0xAABB_CCDD);
        assert_eq!(m.read8(0x1000), 0xDD);
        assert_eq!(m.read16(0x1002), 0xAABB);
        m.write32(0x1FFE, 0x1122_3344);
        assert_eq!(m.read32(0x1FFE), 0x1122_3344);
        assert_eq!(m.read8(0x2000), 0x22);
        m.write16(0x2FFF, 0xBEEF);
        assert_eq!(m.read16(0x2FFF), 0xBEEF);
    }

    #[test]
    fn flags_survive_mapping_and_are_per_page() {
        let mut m = Mem::new();
        m.add_flag(0x6000_0000, 0x6000_1FFF, HOOK_R);
        assert_eq!(m.flags(0x6000_1000), HOOK_R);
        assert_eq!(m.flags(0x6000_2000), 0);
        m.write8(0x6000_0004, 7);
        assert_eq!(m.read8(0x6000_0004), 7);
        assert_eq!(m.flags(0x6000_0004), HOOK_R);
    }

    #[test]
    fn state_hash_ignores_zero_pages_and_tracks_content() {
        let mut a = Mem::new();
        let mut b = Mem::new();
        b.map(0x5000, 0x3000);
        assert_eq!(a.state_hash(), b.state_hash());
        a.write8(0x9001, 1);
        assert_ne!(a.state_hash(), b.state_hash());
        b.write8(0x9001, 1);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn map_keeps_content_and_bytes_helpers() {
        let mut m = Mem::new();
        m.write_bytes(0xC000_0000, &[1, 2, 3, 4, 5]);
        m.map(0xC000_0000, 0x2000);
        assert_eq!(m.read_bytes(0xC000_0000, 5), vec![1, 2, 3, 4, 5]);
        m.zero(0xC000_0001, 2);
        assert_eq!(m.read_bytes(0xC000_0000, 5), vec![1, 0, 0, 4, 5]);
        assert_eq!(m.mapped_pages(), vec![0xC000_0000, 0xC000_1000]);
    }
}
