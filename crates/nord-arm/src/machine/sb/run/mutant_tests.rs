//! OWNER: W2-DRV (tests). Layer L5 for the driver's sites (ids `600..=606`): each mutant is
//! replayed against one scenario of `cycle_tests.rs` and must fail it; the scenarios pass
//! unmutated.

use super::cycle_tests as c;
use super::MUTANTS;
use crate::machine::sb::mutants::{self, with};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// The scenario that must notice mutant `id`.
fn scenario(id: u32) -> Option<fn()> {
    Some(match id {
        600 => c::a_budget_below_n_guest_runs_in_tier_one,
        601 => c::until_and_stop_inside_a_trace_page_are_exact,
        602 => c::stale_scenario,
        603 => c::a_dead_trace_is_never_followed_through_a_link,
        604 => c::a_patch_of_a_traced_page_is_seen_through_the_marks_of_the_trace,
        605 => c::a_fault_inside_a_trace_is_returned_with_the_state_exact,
        606 => c::a_looked_up_successor_is_linked,
        _ => return None,
    })
}

#[test]
fn every_driver_mutant_is_killed_by_its_scenario() {
    let mut killed = 0;
    for (id, name) in MUTANTS {
        let run = scenario(*id).unwrap_or_else(|| panic!("no scenario for mutant {id}: {name}"));
        catch_unwind(AssertUnwindSafe(run))
            .unwrap_or_else(|_| panic!("the scenario of mutant {id} fails unmutated"));
        let failed = catch_unwind(AssertUnwindSafe(|| with(*id, run))).is_err();
        assert!(failed, "mutant {id} survived: {name}");
        killed += 1;
    }
    assert_eq!(killed, MUTANTS.len());
    assert_eq!(mutants::active(), None);
}

#[test]
fn the_ids_are_unique_and_in_the_block_of_this_package() {
    let mut ids: Vec<u32> = MUTANTS.iter().map(|m| m.0).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n);
    assert!(ids.iter().all(|id| (600..=649).contains(id)));
    assert!(scenario(649).is_none());
}
