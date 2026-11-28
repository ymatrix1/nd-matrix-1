//! OWNER: W1-VGEN. The dense budget sweep of design note 8, L1: "budgets 1..7 and 1..400 so the
//! budget expires at every uop position".
//!
//! [`budget_sweep`] turns a range of budgets into schedules, one per budget `b`: a slice of `b`
//! instructions, then a slice of `rest` more. Run from the same machine, the first slice ends
//! after exactly `b` instructions of the program, so over `1..=400` the budget expires at every
//! instruction position of its first 400 instructions, and so at every uop position of every
//! trace built over them; the second slice resumes from that cut, so the state the cut left
//! (flags, `pc`, a trace half done) is compared too, not only the state at the cut.
//! [`sweep_vs_oracle`] runs the sweep against the oracle with a fresh driver per schedule and
//! names the budget that differs.

use super::schedule::{schedule_vs_oracle, SchedStats, Schedule, Slice};
use crate::machine::{Machine, Stop};
use std::ops::RangeInclusive;

/// A slice of `budget` instructions with no `until`, `stop_pc` or hook.
fn plain(budget: u64) -> Slice {
    Slice {
        budget,
        until: 0,
        stop: None,
        hook: None,
    }
}

/// One two-slice schedule (`b` instructions, then `rest`) for each `b` in `budgets`, in order.
pub(crate) fn budget_sweep(budgets: RangeInclusive<u64>, rest: u64) -> Vec<Schedule> {
    budgets
        .map(|b| Schedule {
            slices: vec![plain(b), plain(rest)],
        })
        .collect()
}

/// Run [`budget_sweep`]`(budgets, rest)` from `m0` against the oracle: for each schedule,
/// `new_driver` makes a driver (so a tier with a cache starts every schedule cold) and
/// `schedule_vs_oracle` compares the whole state after both slices. `Ok` returns what the oracle
/// did over all of them (merged), for `assert_exercised`; `Err` names the budget.
pub(crate) fn sweep_vs_oracle<D>(
    m0: &Machine,
    budgets: RangeInclusive<u64>,
    rest: u64,
    mut new_driver: impl FnMut() -> D,
) -> Result<SchedStats, String>
where
    D: FnMut(&mut Machine, u64, u32, Option<u32>) -> (u64, Stop),
{
    let mut stats = SchedStats::default();
    for (b, schedule) in budgets.clone().zip(budget_sweep(budgets, rest)) {
        let mut driver = new_driver();
        let run = schedule_vs_oracle(m0, &schedule, &mut driver)
            .map_err(|e| format!("budget {b}: {e}"))?;
        stats.merge(&run.stats);
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testgen::program::{machine_from, structured_program};
    use crate::machine::sb::testgen::{drive_traces, CODE};
    use crate::machine::sb::testkit::{oracle_run, Rng};
    use std::collections::BTreeSet;

    /// A generated program that runs 600 instructions without a fault or a stop.
    fn long_program(first: u64) -> Machine {
        (first..first + 200)
            .map(|seed| machine_from(&structured_program(&mut Rng::new(seed), 120), false))
            .find(|m| oracle_run(m, 600, 0, None).why == Stop::Budget)
            .expect("some program of 200 runs 600 instructions")
    }

    #[test]
    fn a_sweep_is_one_two_slice_schedule_per_budget_in_order() {
        let all = budget_sweep(1..=400, 500);
        assert_eq!(all.len(), 400);
        for (i, s) in all.iter().enumerate() {
            let budgets: Vec<u64> = s.slices.iter().map(|sl| sl.budget).collect();
            assert_eq!(budgets, vec![i as u64 + 1, 500]);
            assert!(s
                .slices
                .iter()
                .all(|sl| sl.until == 0 && sl.stop.is_none() && sl.hook.is_none()));
        }
        let (lo, hi) = (5, 4); // no budgets at all
        assert!(budget_sweep(lo..=hi, 9).is_empty());
    }

    #[test]
    fn the_first_slice_of_the_sweep_ends_after_every_count_of_instructions() {
        let m = long_program(0);
        let mut cuts = BTreeSet::new();
        for (b, s) in (1..=400u64).zip(budget_sweep(1..=400, 500)) {
            let first = oracle_run(&m, s.slices[0].budget, 0, None);
            assert_eq!((first.n, first.why), (b, Stop::Budget), "budget {b}");
            assert_eq!(first.m.cpu.insns - m.cpu.insns, b);
            cuts.insert(first.m.cpu.r[15]);
        }
        assert!(
            cuts.len() >= 60,
            "the cuts land on {} different pcs: the sweep reaches the program, not one spot",
            cuts.len()
        );
        assert!(cuts.iter().all(|pc| *pc >= CODE));
    }

    #[test]
    fn a_correct_tier_agrees_over_the_sweep_and_the_budget_that_differs_is_named() {
        let m = long_program(0);
        let stats = sweep_vs_oracle(&m, 1..=60, 500, || {
            |m: &mut Machine, n, u, st| m.run(n, u, st)
        })
        .unwrap();
        stats.assert_exercised("T1 over the sweep", 120, 1830 + 60 * 400);
        assert!(
            stats.budget >= 120,
            "every slice ended on its budget: {stats:?}"
        );
        let stats = sweep_vs_oracle(&m, 1..=40, 300, || {
            |m: &mut Machine, n, u, st| drive_traces(m, n, u, st)
        })
        .unwrap();
        stats.assert_exercised("the trace driver over the sweep", 80, 820 + 40 * 250);
        let e = sweep_vs_oracle(&m, 1..=60, 500, || {
            |m: &mut Machine, n, u, st| m.run(if n == 37 { 36 } else { n }, u, st)
        })
        .unwrap_err();
        assert!(
            e.starts_with("budget 37: slice 0") && e.contains("insns"),
            "{e}"
        );
    }

    #[test]
    fn a_fresh_driver_is_made_for_every_schedule() {
        let m = long_program(0);
        let mut made = 0;
        sweep_vs_oracle(&m, 3..=9, 50, || {
            made += 1;
            |m: &mut Machine, n, u, st| m.run(n, u, st)
        })
        .unwrap();
        assert_eq!(made, 7);
    }
}
