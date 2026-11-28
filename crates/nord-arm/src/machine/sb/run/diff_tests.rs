//! OWNER: W2-DRV (tests). Differential tests of the driver against the interpreter: structured
//! programs under random schedules (budgets, `until`/`stop` addresses, hooks added between
//! slices) with one cache that lives across the slices, the dense budget sweep, and flat random
//! programs. Registers, flags, memory, device counters and the instruction count are compared
//! after every slice (`testkit::compare_machines`).
//!
//! Each test ends with the exercised rule: the oracle really ran (`SchedStats`) and the tier
//! really entered traces, dropped some after a store and built again (`SbStats`).

use super::cycle_tests::state;
use super::run_sb_full;
use crate::machine::sb::stats::SbStats;
use crate::machine::sb::testgen::program::{machine_from, structured_program};
use crate::machine::sb::testgen::{
    machine_with, random_program, random_schedule, schedule_vs_oracle, sweep_vs_oracle, tiny_loop,
    SchedStats,
};
use crate::machine::sb::testkit::{compare_machines, fuzz_cases, fuzz_seed, Rng};
use crate::machine::sb::SbConfig;
use crate::machine::Machine;
use std::cell::Cell;
use std::rc::Rc;

/// The tier configurations of the suite: build at once, build after a few dispatches, short
/// traces (so chains and links are everywhere), a build cap of one per call, and a short slice.
fn configs() -> Vec<SbConfig> {
    vec![
        SbConfig::on(),
        SbConfig {
            hot: 3,
            ..SbConfig::on()
        },
        SbConfig {
            max_uops: 6,
            ..SbConfig::on()
        },
        SbConfig {
            builds_per_run: 1,
            slice: 7,
            ..SbConfig::on()
        },
        SbConfig {
            fuse: false,
            predicate: false,
            dead_flags: false,
            fold_literals: false,
            ..SbConfig::on()
        },
    ]
}

fn add(t: &mut SbStats, s: &SbStats) {
    t.built += s.built;
    t.build_failed += s.build_failed;
    t.dropped += s.dropped;
    t.enters += s.enters;
    t.guest_in_traces += s.guest_in_traces;
    t.guest_in_tier1 += s.guest_in_tier1;
    for (a, b) in t.exits.iter_mut().zip(s.exits) {
        *a += b;
    }
}

fn setup(seed: u64) -> (Machine, crate::machine::sb::testgen::Schedule) {
    let mut rng = Rng::new(seed);
    let p = structured_program(&mut rng, 60 + (seed as usize % 7) * 10);
    let m = machine_from(&p, true);
    let s = random_schedule(&mut rng, &p, 8);
    (m, s)
}

#[test]
fn structured_programs_under_random_schedules_equal_the_interpreter() {
    let (mut oracle, mut tier) = (SchedStats::default(), SbStats::default());
    let base = fuzz_seed();
    for (ci, cfg) in configs().into_iter().enumerate() {
        for seed in 0..fuzz_cases(40) {
            let seed = seed ^ base;
            let (m, s) = setup(seed);
            let mut sb = state(cfg.clone());
            let run =
                schedule_vs_oracle(&m, &s, &mut |m, n, u, st| run_sb_full(m, &mut sb, n, u, st))
                    .unwrap_or_else(|e| panic!("config {ci} seed {seed}: {e}"));
            oracle.merge(&run.stats);
            add(&mut tier, &sb.stats);
        }
    }
    oracle.assert_exercised("driver against the interpreter", 1000, 50_000);
    assert!(
        oracle.hooks >= 100 && oracle.until >= 5 && oracle.stop_pc >= 5,
        "{oracle:?}"
    );
    assert!(
        tier.enters >= 1000 && tier.guest_in_traces >= 40_000 && tier.guest_in_tier1 > 0,
        "{tier:?}"
    );
    assert!(
        tier.built >= 300 && tier.dropped >= 20,
        "stores and hooks dropped traces: {tier:?}"
    );
    assert!(
        tier.exits.iter().filter(|n| **n > 0).count() >= 4,
        "several exit kinds were taken: {:?}",
        tier.exits
    );
}

#[test]
fn every_budget_of_the_dense_sweep_equals_the_interpreter() {
    let (mut oracle, mut tier) = (SchedStats::default(), SbStats::default());
    let words = tiny_loop();
    let m0 = machine_with(&words, 7, true);
    for cfg in configs() {
        let entered = Rc::new(Cell::new(0u64));
        let stats = sweep_vs_oracle(&m0, 1..=120, 300, || {
            let (mut sb, entered, mut prev) = (state(cfg.clone()), Rc::clone(&entered), 0);
            move |m: &mut Machine, n, u, st| {
                let res = run_sb_full(m, &mut sb, n, u, st);
                entered.set(entered.get() + sb.stats.enters - prev);
                prev = sb.stats.enters;
                res
            }
        })
        .unwrap();
        oracle.merge(&stats);
        tier.enters += entered.get();
    }
    oracle.assert_exercised("sweep", 1000, 20_000);
    assert!(tier.enters > 1000, "{tier:?}");
}

#[test]
fn flat_random_programs_equal_the_interpreter() {
    let (mut ran, mut tier) = (0u64, SbStats::default());
    for (ci, cfg) in configs().into_iter().enumerate() {
        for case in 0..fuzz_cases(60) {
            let mut rng = Rng::new(fuzz_seed() ^ case.wrapping_mul(0x9E37_79B9));
            let words = random_program(&mut rng, 64);
            let budget = 1 + u64::from(rng.below(600));
            let (mut want, mut got) = (
                machine_with(&words, case, false),
                machine_with(&words, case, true),
            );
            let mut sb = state(cfg.clone());
            let w = want.run(budget, 0, None);
            let g = run_sb_full(&mut got, &mut sb, budget, 0, None);
            compare_machines(&want, w, &got, g)
                .unwrap_or_else(|e| panic!("config {ci} program {case} budget {budget}: {e}"));
            ran += w.0;
            add(&mut tier, &sb.stats);
        }
    }
    assert!(
        ran > 5000 && tier.enters > 100,
        "exercised: {ran} insns, {tier:?}"
    );
}
