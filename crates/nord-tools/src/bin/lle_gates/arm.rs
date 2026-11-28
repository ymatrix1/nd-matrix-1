//! ARM0 contenders of `lle-gates`: the six-phase tick loop of `arm-lle-bench` (`arm_bench::run_all`,
//! per-tick wall time) on one booted machine, cloned for every run. The tiers that exist on the
//! `Machine` today are the plain `Cpu::step` path and the predecode cache; the superblock tiers
//! (`machine::sb`) cannot be selected on a `Machine` until the seam lands, so they are listed as
//! unavailable instead of being measured as something else (hook request in the PR body).

use crate::sched::{Contender, Sample};
use nord_arm::machine::Machine;
use nord_tools::arm_bench::run_all;
use std::rc::Rc;

/// A tier as the command line names it and what it needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// `Cpu::step`, the oracle path (`Machine::predecode = false`).
    Plain,
    /// The predecode cache (the default of `Machine::new`).
    Predecode,
    /// Superblock tier 2 (`NS4_SB`).
    Sb,
    /// Superblock tier 3, native code (`NS4_SB_NATIVE`, feature `sb-native`).
    SbNative,
}

pub const TIERS: [(&str, Tier); 4] = [
    ("plain", Tier::Plain),
    ("predecode", Tier::Predecode),
    ("sb", Tier::Sb),
    ("sb-native", Tier::SbNative),
];

/// Why a tier cannot be measured here, `None` when it can.
pub fn unavailable(tier: Tier) -> Option<&'static str> {
    match tier {
        Tier::Plain | Tier::Predecode => None,
        Tier::Sb => Some("the superblock tier cannot be selected on a Machine yet (seam W4-SEAM not landed)"),
        Tier::SbNative => Some("the native tier cannot be selected on a Machine yet (seam W4-SEAM not landed, feature sb-native)"),
    }
}

/// The run of one tier: all phases at `budget` instructions per tick, ticks in microseconds.
pub fn run_tier(base: &Machine, tier: Tier, budget: u64, scale: usize) -> Result<Sample, String> {
    if let Some(why) = unavailable(tier) {
        return Err(why.into());
    }
    let mut m = base.clone();
    m.predecode = tier == Tier::Predecode;
    let r = run_all(&m, budget, scale);
    Ok(Sample {
        us: r.ms.iter().map(|ms| ms * 1e3).collect(),
        // thread cycles are not microseconds on every platform: no CPU column for the ARM loop
        cpu_us: Vec::new(),
        counters: None,
        steady: true,
    })
}

pub fn contender(
    base: Rc<Machine>,
    name: &str,
    tier: Tier,
    budget: u64,
    scale: usize,
) -> Contender {
    Contender {
        label: name.into(),
        timed: true,
        run: Box::new(move || run_tier(&base, tier, budget, scale)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_superblock_tiers_say_why_they_are_not_measured() {
        assert!(unavailable(Tier::Plain).is_none() && unavailable(Tier::Predecode).is_none());
        assert!(unavailable(Tier::Sb).unwrap().contains("seam"));
        assert!(unavailable(Tier::SbNative).unwrap().contains("sb-native"));
        let m = Machine::new();
        assert!(run_tier(&m, Tier::Sb, 1000, 1)
            .unwrap_err()
            .contains("seam"));
    }

    #[test]
    fn tier_names_are_unique() {
        let mut n: Vec<&str> = TIERS.iter().map(|t| t.0).collect();
        n.sort_unstable();
        n.dedup();
        assert_eq!(n.len(), TIERS.len());
    }
}
