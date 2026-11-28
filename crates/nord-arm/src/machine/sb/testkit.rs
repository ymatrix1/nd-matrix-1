//! OWNER: W1-VGEN (scaffold written by S0; W1-VGEN extends it and keeps every signature).
//! Differential-test kit shared by every work package: a deterministic RNG, biased random
//! states, a logging bus, and full-state snapshots with a readable field diff.
//!
//! Spec (design note 8, WP W1-VGEN): `cfg(test)` support only. The generators and the
//! "lowering versus interpreter" helpers that use this kit live in `testgen.rs`. The comparator
//! must detect every field it claims to compare (one mutated field per test below).
//!
//! `#![allow(dead_code)]`: the kit is consumed by the tests of the other work packages (named in
//! each item's doc), not all by the skeleton's own tests.

#![allow(dead_code)]

use super::bus::SbBus;
use crate::cpu::{Bus, Cpu};
use std::collections::BTreeMap;

mod snapshot;
pub(crate) use snapshot::{snapshot, CpuSnap, Snapshot};

/// xorshift64* with a fixed seed: the same stream on every platform.
#[derive(Clone, Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15 | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn next(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }

    /// True once in `n` on average.
    pub fn chance(&mut self, n: u32) -> bool {
        self.below(n) == 0
    }

    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u32) as usize]
    }

    /// A register value biased to the boundaries that expose flag and rotate bugs, and to
    /// addresses a load or store will hit.
    pub fn biased(&mut self) -> u32 {
        match self.below(8) {
            0 => self.pick(&[
                0,
                1,
                2,
                3,
                0x7FFF_FFFF,
                0x8000_0000,
                0xFFFF_FFFF,
                0xFF,
                0xFFFF,
            ]),
            1 => 0x2000 + self.below(0x100),
            2 => 0x2000 + 4 * self.below(64),
            3 => self.below(40),
            _ => self.next(),
        }
    }
}

/// An environment number (`NS4_FUZZ_CASES`, `NS4_FUZZ_SEED`: the knobs of the existing fuzz
/// tests), or `default`.
pub(crate) fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Cases of a CI-bound fuzz run (`NS4_FUZZ_CASES`, default `default`).
pub(crate) fn fuzz_cases(default: u64) -> u64 {
    env_u64("NS4_FUZZ_CASES", default)
}

/// Seed offset of a fuzz run (`NS4_FUZZ_SEED`, default 0).
pub(crate) fn fuzz_seed() -> u64 {
    env_u64("NS4_FUZZ_SEED", 0)
}

/// A CPU in ARM state at `pc` with biased registers, random flags and `q`, SVC mode.
pub(crate) fn random_cpu(rng: &mut Rng, pc: u32) -> Cpu {
    let mut cpu = Cpu::new();
    for r in cpu.r.iter_mut().take(15) {
        *r = rng.biased();
    }
    cpu.r[15] = pc;
    cpu.set_cpsr(rng.next() & 0xF800_0000 | 0x13);
    cpu.spsr = rng.next();
    cpu
}

/// A bus with an infinite deterministic memory (every byte has a value derived from its
/// address and `salt`, stores overwrite it) that logs every data access as
/// `(kind, addr, value, pc)`; kinds 0 to 2 are loads of 1, 2, 4 bytes and 3 to 5 the stores.
/// Instruction fetches are not logged. `stale` models "a store made decoded code stale".
#[derive(Clone, Default)]
pub(crate) struct LogBus {
    written: BTreeMap<u32, u8>,
    pub log: Vec<(u8, u32, u32, u32)>,
    pub pc: u32,
    pub stale: bool,
    /// A store into `[lo, hi)` sets `stale`.
    pub stale_range: Option<(u32, u32)>,
    salt: u32,
}

impl LogBus {
    pub fn new(salt: u32) -> Self {
        Self {
            salt,
            ..Self::default()
        }
    }

    pub fn peek8(&self, a: u32) -> u8 {
        self.written
            .get(&a)
            .copied()
            .unwrap_or(((a ^ self.salt).wrapping_mul(0x9E37_79B1) >> 24) as u8)
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
        self.log.push((0, a, 0, self.pc));
        self.peek8(a)
    }
    fn read16(&mut self, a: u32) -> u16 {
        self.log.push((1, a, 0, self.pc));
        u16::from_le_bytes([self.peek8(a), self.peek8(a.wrapping_add(1))])
    }
    fn read32(&mut self, a: u32) -> u32 {
        self.log.push((2, a, 0, self.pc));
        self.peek32(a)
    }
    fn write8(&mut self, a: u32, v: u8) {
        self.store(3, a, v as u32, &[v]);
    }
    fn write16(&mut self, a: u32, v: u16) {
        self.store(4, a, v as u32, &v.to_le_bytes());
    }
    fn write32(&mut self, a: u32, v: u32) {
        self.store(5, a, v, &v.to_le_bytes());
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
    fn the_rng_is_deterministic_per_seed_and_differs_between_seeds() {
        let (mut a, mut b, mut c) = (Rng::new(7), Rng::new(7), Rng::new(8));
        let xs: Vec<u32> = (0..8).map(|_| a.next()).collect();
        assert_eq!(xs, (0..8).map(|_| b.next()).collect::<Vec<_>>());
        assert_ne!(xs, (0..8).map(|_| c.next()).collect::<Vec<_>>());
        assert!((0..1000).all(|_| a.below(7) < 7));
        assert!((0..2000).any(|_| a.biased() == 0x8000_0000));
    }

    #[test]
    fn a_random_cpu_is_arm_state_at_pc_in_svc_mode() {
        let mut rng = Rng::new(1);
        for _ in 0..50 {
            let c = random_cpu(&mut rng, 0x1000);
            assert_eq!((c.r[15], c.thumb, c.mode), (0x1000, false, 0x13));
            assert_eq!(c.cpsr() & 0x07FF_FFFF, 0x13, "interrupts enabled, SVC");
        }
    }

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
}
