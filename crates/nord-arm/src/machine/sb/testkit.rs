//! OWNER: W1-VGEN. Differential-test kit shared by every work package: a deterministic RNG,
//! biased random states, a logging bus, an oracle runner, full-state snapshots with a readable
//! field diff, and the [`DiffRun`] counters that prove a differential test exercised something.
//!
//! Spec (design note 8, WP W1-VGEN): `cfg(test)` support only. The generators and the
//! "lowering versus interpreter" helpers that use this kit live in `testgen.rs`. The comparator
//! must detect every field it claims to compare (one mutated field per test in `snapshot`).
//!
//! | Module | What |
//! |---|---|
//! | `state` | [`random_cpu`], [`random_cpu_flags`], [`edge_cpu`], [`random_machine`] |
//! | `logbus` | [`LogBus`]: deterministic memory plus the ordered access log |
//! | `oracle` | [`oracle_run`] (T0 on a clone), [`oracle_trace`] (1 to 3 instructions on a [`LogBus`]), [`compare_runs`] |
//! | `snapshot` | [`snapshot`], [`snapshot_run`], [`Snapshot::diff`], [`mem_diff`] |
//! | `diffrun` | [`DiffRun`], [`Need`]: the "exercised" rule |
//!
//! Every signature of the S0 scaffold is unchanged and the streams of `Rng::next`, `below`,
//! `chance`, `pick`, `biased` and `random_cpu` are bit for bit the scaffold's: work packages that
//! seeded their tests against the skeleton keep their cases. New behaviour is in new items.
//!
//! `#![allow(dead_code)]` (S0's, covers the child modules): the kit is consumed by the tests of
//! the other work packages (named in each item's doc), not all by its own tests.

#![allow(dead_code)]

pub(crate) mod diffrun;
pub(crate) mod logbus;
pub(crate) mod oracle;
pub(crate) mod snapshot;
pub(crate) mod state;

pub(crate) use diffrun::{DiffRun, Need};
pub(crate) use logbus::LogBus;
pub(crate) use oracle::{compare_machines, compare_runs, oracle_run, oracle_trace, Run, TraceRun};
pub(crate) use snapshot::{mem_diff, snapshot, snapshot_run, CpuSnap, Snapshot};
pub(crate) use state::{
    edge_cpu, random_cpu, random_cpu_flags, random_machine, MachineSpec, SHIFT_AMOUNTS,
};

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

    /// Uniform in `lo..hi` (`lo < hi`).
    pub fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + self.below(hi - lo)
    }

    /// True once in `n` on average.
    pub fn chance(&mut self, n: u32) -> bool {
        self.below(n) == 0
    }

    /// A fair coin.
    pub fn flag(&mut self) -> bool {
        self.next() & 1 != 0
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

    /// One of [`SHIFT_AMOUNTS`] (0, 1, 2, 30, 31, 32, 33, 63, 64, 255, 256).
    pub fn shift_amount(&mut self) -> u32 {
        self.pick(&SHIFT_AMOUNTS)
    }

    /// A wider boundary distribution than [`Rng::biased`], for sweeps of the shifter and of
    /// address arithmetic: the shift amounts (two draws in ten), sign, carry and halfword
    /// boundaries, words and unaligned addresses at the end of a data page, and plain randomness.
    pub fn edge(&mut self) -> u32 {
        const BOUNDS: [u32; 14] = [
            0,
            1,
            0x7FFF_FFFF,
            0x8000_0000,
            0x8000_0001,
            0xFFFF_FFFF,
            0xFFFF_FFFE,
            0xFF,
            0x100,
            0x7FFF,
            0x8000,
            0xFFFF,
            0x1_0000,
            0x4000_0000,
        ];
        let page = 0x9000_0000 + (self.below(3) << 12);
        match self.below(10) {
            0 | 1 => self.shift_amount(),
            2 | 3 => self.pick(&BOUNDS),
            4 => page + 0xFFC - 4 * self.below(2),
            5 => page + 0xFFD + self.below(3),
            6 => (page + self.below(0x1000)) | 3,
            7 => page + self.below(0x400) * 4,
            _ => self.next(),
        }
    }

    /// A condition field value: always (14) three times in four, otherwise one of 0..=13.
    pub fn cond(&mut self) -> u32 {
        if self.chance(4) {
            self.below(14)
        } else {
            14
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

    /// The scaffold's stream is part of the contract: tests seeded against the skeleton must keep
    /// their cases (a pinned prefix of the xorshift64* output for seed 1, and of `biased`).
    #[test]
    fn the_scaffold_streams_are_pinned() {
        let mut r = Rng::new(1);
        let first: Vec<u32> = (0..4).map(|_| r.next()).collect();
        assert_eq!(first, [0x7403_D1A2, 0xDD70_F339, 0x7D97_4833, 0x548F_4118]);
        let mut b = Rng::new(1);
        let biased: Vec<u32> = (0..6).map(|_| b.biased()).collect();
        assert_eq!(biased, [0x20E4, 0x10, 0, 0xADD6_51FF, 0x2078, 0x2012]);
    }

    #[test]
    fn the_new_draws_stay_in_range_and_cover_their_menu() {
        let mut r = Rng::new(9);
        assert!((0..500).all(|_| (10..13).contains(&r.range(10, 13))));
        let flags: Vec<bool> = (0..64).map(|_| r.flag()).collect();
        assert!(flags.contains(&true) && flags.contains(&false));
        let shifts: std::collections::BTreeSet<u32> = (0..500).map(|_| r.shift_amount()).collect();
        assert_eq!(shifts, SHIFT_AMOUNTS.into_iter().collect());
        let conds: Vec<u32> = (0..2000).map(|_| r.cond()).collect();
        assert!(conds.iter().all(|c| *c <= 14));
        let al = conds.iter().filter(|c| **c == 14).count();
        assert!((1300..1700).contains(&al), "about 3 in 4 are AL, got {al}");
        assert!(
            (0..14).all(|c| conds.contains(&c)),
            "every other condition appears"
        );
        let edges: Vec<u32> = (0..3000).map(|_| r.edge()).collect();
        for want in [0x8000_0000, 0xFFFF_FFFF, 256, 63, 0x9000_0FFC, 0x9000_1FFC] {
            assert!(edges.contains(&want), "{want:#x} never drawn");
        }
    }

    #[test]
    fn the_environment_knobs_default_when_unset_or_malformed() {
        assert_eq!(env_u64("NS4_NO_SUCH_KNOB_FOR_VGEN", 17), 17);
        std::env::set_var("NS4_VGEN_TEST_KNOB", "x1");
        assert_eq!(env_u64("NS4_VGEN_TEST_KNOB", 5), 5);
        std::env::set_var("NS4_VGEN_TEST_KNOB", "41");
        assert_eq!(env_u64("NS4_VGEN_TEST_KNOB", 5), 41);
        std::env::remove_var("NS4_VGEN_TEST_KNOB");
        assert!(fuzz_cases(60) >= 1);
        let _ = fuzz_seed();
    }

    /// Every item the hub re-exports is the one its module defines (the paths other work
    /// packages import).
    #[test]
    fn the_hub_re_exports_the_kit() {
        let mut rng = Rng::new(3);
        let cpu = random_cpu_flags(&mut rng, 0x1000, 5);
        let edge = edge_cpu(&mut rng, 0x1000);
        assert_eq!((cpu.nzcv(), edge.r[15]), (5, 0x1000));
        let m = random_machine(&mut rng, &[0xE1A0_0000], &MachineSpec::default());
        let run: Run = oracle_run(&m, 1, 0, None);
        let snap: Snapshot = snapshot(&m, None);
        assert_eq!((run.n, snap.cpu.r[15]), (1, 0xC000_0000));
        assert_eq!(compare_runs(&run, &run), Ok(()));
        assert_eq!(mem_diff(&m.bus.mem, &m.bus.mem, 4), Vec::new());
        let tr: TraceRun = oracle_trace(&cpu, &[0xE1A0_0000], 1, 1);
        assert_eq!(tr.executed, 1);
        let (mut d, _): (DiffRun, Need) = (DiffRun::default(), Need::default());
        d.cases += 1;
        let _: (CpuSnap, LogBus) = (snap.cpu.clone(), LogBus::new(0));
        assert_eq!(snapshot_run(&m, &run.why).stop, Some(run.why));
    }
}
