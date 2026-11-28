//! OWNER: W2-NDP (tests). Layer L5 for the native data-processing lowering: every mutant of
//! [`MUTANTS`] (ids `800..=849`, planted as `mutant(id)` sites in the lowering) must make the
//! reduced differential suite [`quick`] fail. A mutant that survives is a hole in the suite, so
//! the test names the survivors. Mutants change what is lowered, so the suite compiles inside
//! `mutants::with` on this thread; where native code is not available there is nothing to run
//! and the test prints why and returns (the sites are plain code, covered by the IR-level
//! tests of the other files).

use super::harness::Rig;
use super::sweeps::{
    sweep_arith_edges, sweep_generic, sweep_immediates, sweep_pred, sweep_shift_imm,
    sweep_shift_reg, sweep_specials, VALS,
};
use super::MUTANTS;
use crate::machine::sb::mutants;
use crate::machine::sb::testkit::Rng;

/// The reduced differential suite: each sweep at a size that still reaches every boundary the
/// mutants move (about 130 compiled functions, a few hundred thousand comparisons).
pub(super) fn quick(rig: &mut Rig) -> Result<(), String> {
    let mut rng = Rng::new(0x4E44_5100);
    sweep_specials(rig, &mut rng, 1, 24)?;
    sweep_shift_imm(rig, &mut rng, &[0, 1, 2, 3], &[0, 1, 2, 31])?;
    sweep_shift_reg(
        rig,
        &mut rng,
        &[0, 1, 2, 3],
        &[0, 1, 31, 32, 33, 64, 255, 256, 257],
    )?;
    let edge = [0, 1, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, VALS[6]];
    sweep_arith_edges(rig, &mut rng, &[2, 3, 4, 5, 6, 7, 10, 11], &edge)?;
    sweep_immediates(rig, &mut rng, &[0, 1, 4], &[0, 13, 5])?;
    sweep_pred(rig, &mut rng, &[8, 10, 11, 14])?;
    sweep_generic(rig, &mut rng, 0..96, 1, 3)?;
    Ok(())
}

#[test]
fn the_reduced_suite_passes_without_a_mutant() {
    let Some(mut rig) = Rig::new() else { return };
    assert_eq!(mutants::active(), None);
    quick(&mut rig).unwrap();
}

#[test]
fn every_native_dp_mutant_is_killed_by_the_differential_suite() {
    let Some(mut rig) = Rig::new() else { return };
    let mut survivors = Vec::new();
    for &(id, name) in MUTANTS {
        let killed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mutants::with(id, || quick(&mut rig))
        }))
        .map_or(true, |r| r.is_err());
        if !killed {
            survivors.push((id, name));
        }
    }
    assert_eq!(mutants::active(), None, "the switch is restored");
    assert!(survivors.is_empty(), "surviving mutants: {survivors:?}");
    assert!(MUTANTS.len() >= 25, "at least 25 sites in this block");
}
