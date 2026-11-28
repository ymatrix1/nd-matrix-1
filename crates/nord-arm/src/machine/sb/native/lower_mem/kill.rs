//! OWNER: W2-NMEM. Kill suite of the native memory and flow mutants (`850..=893`), `cfg(test)`.

use super::harness::*;
use super::suites::{block_suite, shift_and_abs_suite};
use super::tests::{sweep, trace_suite};
use super::MUTANTS;
use crate::machine::sb::mutants;
use crate::machine::sb::testkit::Rng;

/// Run the kill suite once: false when the oracle noticed a divergence (or the mutated
/// lowerer panicked).
fn kill_suite_passes() -> bool {
    let Some(mut jit) = native_or_skip("kill_suite") else {
        return true;
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut rng = Rng::new(0x4B11);
        sweep(&mut jit, &mut rng, &MEM_KINDS, 6, 12)?;
        trace_suite(&mut jit, &mut rng, 30, 8)?;
        block_suite(&mut jit, &mut rng, 4)?;
        shift_and_abs_suite(&mut jit, &mut rng)?;
        super::super::lower_flow::suite::kill_suite(&mut jit)
    }));
    matches!(result, Ok(Ok(_)))
}

#[test]
fn every_native_memory_and_flow_mutant_is_killed_and_the_baseline_passes() {
    if crate::machine::sb::native::available() {
        assert!(
            kill_suite_passes(),
            "the unmutated lowerer must pass its own suite"
        );
    } else {
        println!("skipped the mutation check: native code is unavailable on this host");
        return;
    }
    let mut survivors = Vec::new();
    for &(id, name) in MUTANTS
        .iter()
        .chain(super::super::lower_flow::MUTANTS.iter())
    {
        if kill_suite_passes_with(id) {
            survivors.push((id, name));
        }
    }
    assert!(survivors.is_empty(), "surviving mutants: {survivors:?}");
}

fn kill_suite_passes_with(id: u32) -> bool {
    mutants::with(id, kill_suite_passes)
}

#[test]
fn the_mutants_are_listed_once_inside_the_reserved_block() {
    let mut ids: Vec<u32> = MUTANTS
        .iter()
        .chain(super::super::lower_flow::MUTANTS.iter())
        .map(|m| m.0)
        .collect();
    let all = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), all, "no id twice");
    assert!(ids.iter().all(|id| (850..=899).contains(id)));
    assert!(mutants::active().is_none());
    assert!(
        MUTANTS.iter().all(|(id, _)| !super::mutant(*id)),
        "off outside a mutant run"
    );
}

#[test]
fn the_registry_in_mutants_rs_lists_exactly_the_sites_of_the_two_modules() {
    let mut own: Vec<(u32, &str)> = MUTANTS
        .iter()
        .chain(super::super::lower_flow::MUTANTS.iter())
        .copied()
        .collect();
    own.sort_unstable();
    let mut registered: Vec<(u32, &str)> = mutants::MUTANTS
        .iter()
        .filter(|(id, _)| (850..=899).contains(id))
        .copied()
        .collect();
    registered.sort_unstable();
    assert_eq!(registered, own);
}
