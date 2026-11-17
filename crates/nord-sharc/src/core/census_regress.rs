//! Regression fixture of the firmware-instruction census diff.
//!
//! VERIFIED: `census_measured.txt` holds the CCES 2.12.1 ADSP-21489 simulator's output for every case
//! of `census_cases.txt` (`docs/research/sharc-seq/census_regress.py` picks them from a full
//! `census_diff.py` run): one instruction per firmware shape that ever showed a model-versus-simulator
//! difference (long-word moves, SIMD register and memory moves, per-element conditions, shifter flags,
//! sticky and ASTAT writes, multifunction flags ...) with six random core states each, plus a sweep of
//! every condition code over all combinations of AZ AV AN AC and AF. Every line is the dumped state
//! (107 words) and the scratch words the instruction changed.
use super::census::{compact, run_cases, scratch_of};

const FIXTURE: &str = include_str!("../../../../docs/research/sharc-seq/census_fixture.txt");
const CASES: &str = include_str!("../../../../docs/research/sharc-seq/census_cases.txt");
const MEASURED: &str = include_str!("../../../../docs/research/sharc-seq/census_measured.txt");
const N_STATE: usize = 107;

/// Replays `cases` on the model and compares with the simulator output `measured`.
fn check(cases: &str, measured: &str) {
    let scr = scratch_of(cases);
    let got = run_cases(FIXTURE, cases).unwrap();
    let want: Vec<&str> = measured.lines().collect();
    let got: Vec<String> = got.lines().map(|l| compact(l, &scr, N_STATE)).collect();
    assert_eq!(got.len(), want.len(), "one output line per case");
    let cases: Vec<&str> = cases.lines().skip(1).collect();
    let bad: Vec<String> = (0..got.len())
        .filter(|&i| got[i] != want[i])
        .map(|i| {
            format!(
                "case {i} ({}): got {} want {}",
                &cases[i][..12],
                &got[i][..40],
                &want[i][..40]
            )
        })
        .collect();
    let shown = bad[..bad.len().min(20)].join(
        "
",
    );
    assert!(
        bad.is_empty(),
        "{} cases differ (first shown):
{shown}",
        bad.len()
    );
}

#[test]
fn model_matches_the_simulator_on_every_census_regression_case() {
    check(CASES, MEASURED);
}

const EXTRA_CASES: &str =
    include_str!("../../../../docs/research/sharc-seq/census_extra_cases.txt");
const EXTRA_MEASURED: &str =
    include_str!("../../../../docs/research/sharc-seq/census_extra_measured.txt");

/// VERIFIED on the ADSP-21489 simulator (`census_probe.py`, 461 cases): the former census-diff
/// gaps. Shift-immediate counts (8-bit signed: `BY 256` shifts by 0, 225 cases), the conditions
/// MV and MS and the per-element swap `IF cc R14 <-> S3` (all 32 condition codes x 6 states,
/// 180 cases), `STKYx/STKYy/ASTATx/ASTATy = DM(a) (LW)` (16 cases) and `IF cc M12 = DM(I0, M0)`
/// in SIMD mode (40 cases). Each of these failed on the model before the fix.
#[test]
fn model_matches_the_simulator_on_the_closed_census_diffs() {
    check(EXTRA_CASES, EXTRA_MEASURED);
}

const PLUS_FIXTURE: &str = include_str!("../../../../docs/research/sharc-seq/census21_fixture.txt");
const PLUS_CASES: &str = include_str!("../../../../docs/research/sharc-seq/plus_cases.txt");
const PLUS_MEASURED: &str = include_str!("../../../../docs/research/sharc-seq/plus_measured.txt");

/// VERIFIED on the ADSP-21571 cycle-accurate simulator: 120 random full core states written by the
/// loader and read back after a no-op (ASTATx/y, STKYx/y write masks of SHARC+).
#[test]
fn sharc_plus_system_register_masks_match_the_21571_simulator() {
    let scr = scratch_of(PLUS_CASES);
    let got = run_cases(PLUS_FIXTURE, PLUS_CASES).unwrap();
    let want: Vec<&str> = PLUS_MEASURED.lines().collect();
    let got: Vec<String> = got.lines().map(|l| compact(l, &scr, N_STATE)).collect();
    assert_eq!(got.len(), want.len());
    let bad = (0..got.len()).filter(|&i| got[i] != want[i]).count();
    assert_eq!(bad, 0, "{bad} of {} cases differ", got.len());
}
