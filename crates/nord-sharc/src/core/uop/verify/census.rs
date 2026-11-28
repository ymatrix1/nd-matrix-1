//! Census replay: every case of the measured census on the uop engine against the interpreter.
//!
//! OWNER: W3-FUZZ. The cases are the 1,580 hardware-measured single-instruction probes of
//! `docs/research/sharc-seq/` (the census of D152, also replayed on the JIT in
//! `core/census_engines.rs`). Gate G1 needs the dumped state of every case to be word for word
//! the interpreter's. Skeleton: complete; with all-`Interp` lowering it passes by construction.
use crate::core::census::{run_cases_on, Engine};

const FIXTURE: &str = include_str!("../../../../../../docs/research/sharc-seq/census_fixture.txt");
const CASES: &str = include_str!("../../../../../../docs/research/sharc-seq/census_cases.txt");
const EXTRA_CASES: &str =
    include_str!("../../../../../../docs/research/sharc-seq/census_extra_cases.txt");

/// Replays the census cases on `engine` and on the interpreter and compares the output line by
/// line. Returns the number of cases compared, or a description of the first difference.
pub fn replay(engine: Engine) -> Result<usize, String> {
    let mut total = 0;
    for cases in [CASES, EXTRA_CASES] {
        let want = run_cases_on(Engine::Interp, FIXTURE, cases)?;
        let got = run_cases_on(engine, FIXTURE, cases)?;
        if want.lines().count() != got.lines().count() {
            return Err(format!(
                "{} cases on the interpreter, {} on {engine:?}",
                want.lines().count(),
                got.lines().count()
            ));
        }
        for (k, (w, g)) in want.lines().zip(got.lines()).enumerate() {
            if w != g {
                return Err(format!("case {k} differs on {engine:?}: {g} vs {w}"));
            }
            total += 1;
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_census_case_on_the_uop_engine_equals_the_interpreter() {
        let n = replay(Engine::Uop).unwrap();
        assert!(n >= 1500, "{n} cases replayed");
    }

    #[test]
    fn the_checked_engine_variant_replays_the_same_cases() {
        assert!(replay(Engine::UopChecked).unwrap() >= 1500);
    }

    #[test]
    fn the_interpreter_against_itself_is_trivially_equal() {
        assert_eq!(
            replay(Engine::Interp).unwrap(),
            replay(Engine::Interp).unwrap()
        );
    }
}
