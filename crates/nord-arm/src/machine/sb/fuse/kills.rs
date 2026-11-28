//! OWNER: W2-PEEP (tests). Mutation kill tests of layer L5 for the three peephole passes (ids
//! `500..=599`: `flagopt` 500..=529, `fuse` 530..=559, `pred` 560..=589).
//!
//! A mutant is killed when a suite of this package fails under it: the suites are replayed whole
//! under `mutants::with(id, ..)` and one failed assertion is a kill. They run in order of cost;
//! the cheap ones pin the decisions of each pass on hand-built input, the differential probes
//! must also see every mutant that changes state (so the harness itself is proven to notice
//! them), and the full differential suites are the last resort.

use crate::machine::sb::flagopt::{self, effect, MUTANTS as DEAD_MUTANTS};
use crate::machine::sb::fuse::{self, MUTANTS as FUSE_MUTANTS};
use crate::machine::sb::mutants::{self, with};
use crate::machine::sb::pred::{self, MUTANTS as PRED_MUTANTS};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// The suites in order of cost.
const SUITES: [(&str, fn()); 10] = [
    ("effect", effect::tests::run_all),
    ("flagopt cases", flagopt::cases::run_all),
    ("pred cases", pred::cases::run_all),
    ("fuse cases", fuse::cases::run_all),
    ("fuse probe", fuse::diff::probe),
    ("flagopt probe", flagopt::diff::probe),
    ("pred probe", pred::diff::probe),
    ("flagopt diff", flagopt::diff::run_all),
    ("pred diff", pred::diff::run_all),
    ("fuse diff", fuse::diff::run_all),
];

/// The first of `suites` (names) that fails under mutant `id`.
fn killer(id: u32, suites: &[&str]) -> Option<&'static str> {
    SUITES
        .iter()
        .filter(|(name, _)| suites.contains(name))
        .find(|(_, run)| catch_unwind(AssertUnwindSafe(|| with(id, run))).is_err())
        .map(|(name, _)| *name)
}

fn all_mutants() -> impl Iterator<Item = &'static (u32, &'static str)> {
    DEAD_MUTANTS
        .iter()
        .chain(FUSE_MUTANTS.iter())
        .chain(PRED_MUTANTS.iter())
}

fn every_suite() -> Vec<&'static str> {
    SUITES.iter().map(|(name, _)| *name).collect()
}

#[test]
fn every_peephole_mutant_is_killed_by_a_suite_of_this_package() {
    let suites = every_suite();
    let survivors: Vec<_> = all_mutants()
        .filter(|(id, _)| killer(*id, &suites).is_none())
        .collect();
    assert!(survivors.is_empty(), "survivors: {survivors:?}");
    assert_eq!(
        mutants::active(),
        None,
        "a killed mutant leaves nothing switched on"
    );
}

#[test]
fn the_unmutated_suites_pass_and_the_mutant_ids_are_unique_and_in_the_block() {
    for (name, run) in SUITES {
        if name.ends_with("diff") {
            continue; // run as tests of their own
        }
        catch_unwind(AssertUnwindSafe(run))
            .unwrap_or_else(|_| panic!("suite {name} fails unmutated"));
    }
    assert_eq!(
        killer(599, &every_suite()),
        None,
        "a site that does not exist survives"
    );
    let mut ids: Vec<u32> = all_mutants().map(|(id, _)| *id).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "ids are unique across the three lists");
    assert!(ids.iter().all(|id| (500..=599).contains(id)));
    assert!(n >= 30, "{n} mutants");
}

#[test]
fn the_mutants_that_change_state_are_also_seen_by_the_differential_probes() {
    // the flag mutants change what the executor ends up with, so the L0 probe must see them
    // too, not only the decision tests: that is what proves the harness compares the flags.
    // The rest (barrier, switch, cap and body-shape decisions) are pinned by the case suites.
    let probes = ["fuse probe", "flagopt probe", "pred probe"];
    for id in [500, 501, 502, 503, 504, 507, 508, 509, 512, 513, 515] {
        assert!(killer(id, &probes).is_some(), "flagopt mutant {id}");
    }
    for id in [531, 532, 534, 535, 536, 537, 538, 539] {
        assert!(killer(id, &probes).is_some(), "fuse mutant {id}");
    }
    for id in [560, 561, 562, 564] {
        assert!(killer(id, &probes).is_some(), "pred mutant {id}");
    }
}
