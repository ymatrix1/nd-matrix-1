//! OWNER: W2-XCORE (tests). Mutation kill test of layer L5 for the sites of `exec.rs` (ids
//! `450..=458`) and `exec_flow.rs` with its children (ids `400..=421`).
//!
//! A mutant is killed when a suite of this work package fails under it: the suites are replayed
//! whole under `mutants::with(id, ..)` and one failed assertion is a kill. They run in order of
//! cost; a mutant that no suite notices is a hole in the oracle.

use crate::machine::sb::exec::{exits, stores, MUTANTS};
use crate::machine::sb::exec_flow::{cases, fused, psr, MUTANTS as FLOW_MUTANTS};
use crate::machine::sb::mutants::{self, with};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// The suites in order of cost.
const SUITES: [(&str, fn()); 5] = [
    ("cases", cases::run_all),
    ("exits", exits::run_all),
    ("psr", psr::tests::run_all),
    ("fused", fused::tests::run_all),
    ("stores", stores::run_all),
];

/// The first suite that fails under mutant `id`.
fn killer(id: u32) -> Option<&'static str> {
    SUITES
        .iter()
        .find(|(_, run)| catch_unwind(AssertUnwindSafe(|| with(id, run))).is_err())
        .map(|(name, _)| *name)
}

fn all_mutants() -> impl Iterator<Item = &'static (u32, &'static str)> {
    FLOW_MUTANTS.iter().chain(MUTANTS.iter())
}

#[test]
fn every_executor_mutant_is_killed_by_a_suite_of_this_package() {
    let survivors: Vec<_> = all_mutants()
        .filter(|(id, _)| killer(*id).is_none())
        .collect();
    assert!(survivors.is_empty(), "survivors: {survivors:?}");
    assert_eq!(
        mutants::active(),
        None,
        "a killed mutant leaves nothing switched on"
    );
}

#[test]
fn the_unmutated_suites_pass_and_a_site_that_does_not_exist_is_never_killed() {
    for (name, run) in SUITES {
        catch_unwind(AssertUnwindSafe(run))
            .unwrap_or_else(|_| panic!("suite {name} fails unmutated"));
    }
    assert_eq!(killer(499), None);
    let mut ids: Vec<u32> = all_mutants().map(|(id, _)| *id).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "ids are unique across both lists");
    assert!(ids.iter().all(|id| (400..=499).contains(id)));
    assert!(
        n >= 30,
        "design note 8: the executors carry their share of the 40 mutants"
    );
}

#[test]
fn the_mutants_of_the_exit_and_of_the_loop_are_caught_by_the_hand_built_traces() {
    // `exec_flow` alone cannot see `finish`: only `run_sb` on whole traces does
    for id in [450, 451, 452, 453, 454, 455, 456, 457] {
        assert_eq!(killer(id), Some("exits"), "mutant {id}");
    }
    // every store kind must end the trace, not only the two the hand-built traces use
    assert_eq!(killer(458), Some("stores"));
    // the arms are pinned one by one before any trace runs
    for id in [400, 401, 402, 403, 404, 406, 407, 408] {
        assert_eq!(killer(id), Some("cases"), "mutant {id}");
    }
}
