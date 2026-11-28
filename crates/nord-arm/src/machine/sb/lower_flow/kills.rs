//! OWNER: W1-FLOW (tests). Mutation kill tests of layer L5 for the sites `FLOW_MUTANTS` lists
//! (ids `200..=216`, `lower_flow.rs`, `trace.rs`, `trace/parts.rs`).
//!
//! A mutant is killed when a suite of this work package fails under it: the suites are replayed
//! whole under `mutants::with(id, ..)` and one failed assertion is a kill. The suites run in order
//! of cost (`lower_flow` cases, trace parts, trace shapes, and `diff::probe`, a short differential
//! run against the interpreter); a mutant must die in the cheap ones, which pin the static facts,
//! and the state mutants must also die in the differential run, which is what proves the oracle
//! sees them. The full differential suite runs unmutated as its own tests.

use super::{cases, FLOW_MUTANTS};
use crate::machine::sb::mutants::{self, with};
use crate::machine::sb::trace::{diff, parts, shape, shape_caps};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// The suites in order of cost.
const SUITES: [(&str, fn()); 5] = [
    ("cases", cases::run_all),
    ("parts", parts::run_all),
    ("shape", shape::run_all),
    ("caps", shape_caps::run_all),
    ("diff", diff::probe),
];

/// The first of `suites` (names) that fails under mutant `id`.
fn killer(id: u32, suites: &[&str]) -> Option<&'static str> {
    SUITES
        .iter()
        .filter(|(name, _)| suites.contains(name))
        .find(|(_, run)| catch_unwind(AssertUnwindSafe(|| with(id, run))).is_err())
        .map(|(name, _)| *name)
}

#[test]
fn every_flow_mutant_dies_in_the_cheap_suites() {
    let survivors: Vec<_> = FLOW_MUTANTS
        .iter()
        .filter(|(id, _)| killer(*id, &["cases", "parts", "shape", "caps"]).is_none())
        .collect();
    assert!(survivors.is_empty(), "survivors: {survivors:?}");
    assert_eq!(
        mutants::active(),
        None,
        "a killed mutant leaves nothing switched on"
    );
}

#[test]
fn the_differential_run_alone_sees_the_mutants_that_change_the_state() {
    for id in [200, 201, 202, 203, 204, 206, 210, 211] {
        assert_eq!(killer(id, &["diff"]), Some("diff"), "mutant {id}");
    }
}

#[test]
fn the_count_of_a_followed_branch_is_pinned_by_the_shape_suite_through_the_sum_of_n() {
    // the no-op changes no state (`n` is bookkeeping), so only the L6 check on `Uop::n` sees it
    assert_eq!(killer(216, &["cases", "parts", "shape"]), Some("shape"));
}

#[test]
fn a_site_that_does_not_exist_is_never_killed_and_every_suite_passes_unmutated() {
    assert_eq!(
        killer(299, &["cases", "parts", "shape", "caps", "diff"]),
        None
    );
    let mut ids: Vec<u32> = FLOW_MUTANTS.iter().map(|(id, _)| *id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), FLOW_MUTANTS.len(), "ids are unique");
    assert!(ids.iter().all(|id| (200..=299).contains(id)));
}
