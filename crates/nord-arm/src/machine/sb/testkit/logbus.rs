//! OWNER: W1-VGEN. The logging bus every executor and lowering test runs against: an infinite
//! deterministic memory that records the ordered data accesses, the pc each one was made at, and
//! whether a store made decoded code stale.
//!
//! Spec (design note 8, L0): "against `Cpu::step` on a logging bus (registers, flags, bus access
//! log)". Two runs on equal `LogBus` values that perform the same accesses in the same order and
//! at the same `pc` leave equal logs, so a log comparison proves I5 (memory effects in guest
//! order, with `bus.pc` set to the accessing instruction).

use crate::cpu::Bus;
use crate::machine::sb::bus::SbBus;
use std::collections::BTreeMap;

/// Access kinds of the log (`LogBus::log.0`).
pub(crate) mod kind {
    pub const LOAD8: u8 = 0;
    pub const LOAD16: u8 = 1;
    pub const LOAD32: u8 = 2;
    pub const STORE8: u8 = 3;
    pub const STORE16: u8 = 4;
    pub const STORE32: u8 = 5;

    /// True for the three store kinds.
    pub const fn is_store(k: u8) -> bool {
        k >= STORE8
    }
}

/// A bus with an infinite deterministic memory (every byte has a value derived from its
/// address and `fill`, stores overwrite it) that logs every data access as
/// `(kind, addr, value, pc)`; kinds 0 to 2 are loads of 1, 2, 4 bytes and 3 to 5 the stores
/// (see [`kind`]). The value of a load is not logged (it is a function of the stores before it).
/// Instruction fetches are not logged. `stale` models "a store made decoded code stale".
#[derive(Clone, Default)]
pub(crate) struct LogBus {
    written: BTreeMap<u32, u8>,
    pub log: Vec<(u8, u32, u32, u32)>,
    pub pc: u32,
    pub stale: bool,
    /// A store into `[lo, hi)` sets `stale`.
    pub stale_range: Option<(u32, u32)>,
    fill: u32,
}

impl LogBus {
    pub fn new(fill: u32) -> Self {
        Self {
            fill,
            ..Self::default()
        }
    }

    pub fn peek8(&self, a: u32) -> u8 {
        self.written
            .get(&a)
            .copied()
            .unwrap_or(((a ^ self.fill).wrapping_mul(0x9E37_79B1) >> 24) as u8)
    }

    pub fn peek32(&self, a: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|i| self.peek8(a.wrapping_add(i))))
    }

    /// Store without logging (test setup).
    pub fn poke32(&mut self, a: u32, v: u32) {
        for (i, b) in v.to_le_bytes().iter().enumerate() {
            self.written.insert(a.wrapping_add(i as u32), *b);
        }
    }

    /// Place `words` at `at`, one per 4 bytes, without logging (a program for the interpreter).
    pub fn load_words(&mut self, at: u32, words: &[u32]) {
        for (i, w) in words.iter().enumerate() {
            self.poke32(at.wrapping_add(4 * i as u32), *w);
        }
    }

    /// Number of logged loads.
    pub fn loads(&self) -> usize {
        self.log.iter().filter(|e| !kind::is_store(e.0)).count()
    }

    /// Number of logged stores.
    pub fn stores(&self) -> usize {
        self.log.iter().filter(|e| kind::is_store(e.0)).count()
    }

    /// A readable first difference of two logs (kind, address, value, pc), or `None` when equal.
    pub fn diff(&self, o: &Self) -> Option<String> {
        let name = |k: u8| ["ld8", "ld16", "ld32", "st8", "st16", "st32"][k as usize % 6];
        let show = |e: &(u8, u32, u32, u32)| {
            format!("{} [{:#x}] = {:#x} at pc {:#x}", name(e.0), e.1, e.2, e.3)
        };
        let n = self.log.len().min(o.log.len());
        if let Some(i) = (0..n).find(|i| self.log[*i] != o.log[*i]) {
            return Some(format!(
                "access {i}: {} != {}",
                show(&self.log[i]),
                show(&o.log[i])
            ));
        }
        (self.log.len() != o.log.len()).then(|| {
            let (long, who) = if self.log.len() > n {
                (&self.log[n], "left")
            } else {
                (&o.log[n], "right")
            };
            format!(
                "access log lengths {} != {}: extra access on the {who}: {}",
                self.log.len(),
                o.log.len(),
                show(long)
            )
        })
    }

    fn store(&mut self, kind: u8, a: u32, v: u32, bytes: &[u8]) {
        self.log.push((kind, a, v, self.pc));
        for (i, b) in bytes.iter().enumerate() {
            self.written.insert(a.wrapping_add(i as u32), *b);
        }
        if self
            .stale_range
            .is_some_and(|(lo, hi)| (lo..hi).contains(&a))
        {
            self.stale = true;
        }
    }
}

impl Bus for LogBus {
    fn read8(&mut self, a: u32) -> u8 {
        self.log.push((kind::LOAD8, a, 0, self.pc));
        self.peek8(a)
    }
    fn read16(&mut self, a: u32) -> u16 {
        self.log.push((kind::LOAD16, a, 0, self.pc));
        u16::from_le_bytes([self.peek8(a), self.peek8(a.wrapping_add(1))])
    }
    fn read32(&mut self, a: u32) -> u32 {
        self.log.push((kind::LOAD32, a, 0, self.pc));
        self.peek32(a)
    }
    fn write8(&mut self, a: u32, v: u8) {
        self.store(kind::STORE8, a, v as u32, &[v]);
    }
    fn write16(&mut self, a: u32, v: u16) {
        self.store(kind::STORE16, a, v as u32, &v.to_le_bytes());
    }
    fn write32(&mut self, a: u32, v: u32) {
        self.store(kind::STORE32, a, v, &v.to_le_bytes());
    }
    fn fetch16(&mut self, a: u32) -> u16 {
        u16::from_le_bytes([self.peek8(a), self.peek8(a.wrapping_add(1))])
    }
    fn fetch32(&mut self, a: u32) -> u32 {
        self.peek32(a)
    }
}

impl SbBus for LogBus {
    fn set_pc(&mut self, pc: u32) {
        self.pc = pc;
    }
    fn stale(&self) -> bool {
        self.stale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_bus_serves_deterministic_bytes_logs_in_order_and_flags_stale_stores() {
        let mut b = LogBus::new(5);
        let first = b.read32(0x1000);
        assert_eq!(first, LogBus::new(5).peek32(0x1000));
        b.pc = 0xC000_0004;
        b.write8(0x1001, 0xAB);
        assert_eq!(b.read32(0x1000), first & 0xFFFF_00FF | 0xAB00);
        assert_eq!(
            b.log,
            vec![
                (2, 0x1000, 0, 0),
                (3, 0x1001, 0xAB, 0xC000_0004),
                (2, 0x1000, 0, 0xC000_0004)
            ]
        );
        assert!(!b.stale);
        b.stale_range = Some((0x2000, 0x3000));
        b.write32(0x1FFC, 1);
        assert!(!b.stale);
        b.write16(0x2800, 1);
        assert!(SbBus::stale(&b));
        b.poke32(0x4000, 9);
        assert_eq!((b.peek32(0x4000), b.log.len()), (9, 5), "poke does not log");
    }

    #[test]
    fn the_fill_changes_the_contents_and_every_access_size_logs_its_kind() {
        let (mut a, mut b) = (LogBus::new(1), LogBus::new(2));
        assert_ne!(a.peek32(0x2000), b.peek32(0x2000));
        a.read8(1);
        a.read16(2);
        a.read32(4);
        a.write8(8, 1);
        a.write16(10, 2);
        a.write32(12, 3);
        let kinds: Vec<u8> = a.log.iter().map(|e| e.0).collect();
        assert_eq!(kinds, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!((a.loads(), a.stores()), (3, 3));
        assert_eq!(b.fetch32(0x40), b.peek32(0x40));
        assert_eq!(b.fetch16(0x42) as u32, b.peek32(0x40) >> 16);
        assert!(b.log.is_empty(), "a fetch is not a data access");
    }

    #[test]
    fn load_words_places_words_and_diff_names_the_first_difference() {
        let mut a = LogBus::new(0);
        a.load_words(0x100, &[0xE3A0_0001, 0xE3A0_1002]);
        assert_eq!(
            (a.peek32(0x100), a.peek32(0x104)),
            (0xE3A0_0001, 0xE3A0_1002)
        );
        assert!(a.log.is_empty());
        let (mut x, mut y) = (a.clone(), a.clone());
        assert_eq!(x.diff(&y), None);
        x.pc = 0x100;
        y.pc = 0x100;
        x.write32(0x200, 5);
        y.write32(0x200, 6);
        let d = x.diff(&y).unwrap();
        assert!(
            d.contains("access 0") && d.contains("st32 [0x200] = 0x5"),
            "{d}"
        );
        y.log = x.log.clone();
        y.write8(0x300, 1);
        let d = x.diff(&y).unwrap();
        assert!(d.contains("lengths 1 != 2") && d.contains("right"), "{d}");
        assert!(y.diff(&x).unwrap().contains("left"));
    }
}
