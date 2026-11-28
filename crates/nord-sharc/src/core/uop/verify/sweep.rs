//! Exit sweep: force an exit at every possible step boundary and check the continuation.
//!
//! OWNER: W3-FUZZ. Design 6.7: for a program of `n` instructions, run the engine for `k` steps
//! (so every block that crosses step `k` exits at the nearest uop boundary through the checked
//! loop and `Core::step`), then run it for the remaining `n - k`, and compare with `n` steps of
//! the interpreter, for every `k` in `0..=n`. This exercises the exit protocol (materialization
//! at a uop boundary, resume through the pc map) on every block of every program, not only on
//! the exits a single run happens to take. Skeleton: complete.
use super::compare::{exact_diff, ExactDiff};
use super::gen::{core_with_code, fill_state, Rng, CODE_PC};
use crate::core::uop::config::{UopConfig, UopError};

/// Why a sweep stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SweepError {
    /// The engine could not be armed on the program.
    Engine(UopError),
    /// Splitting the run after `split` steps made the result differ from the interpreter's.
    Diff { split: u64, diff: ExactDiff },
}

/// Sweeps every split `0..=insns` of a straight-line `program` of `insns` instructions on the
/// state of `seed`. Returns the number of splits checked.
pub fn exit_sweep(
    program: &[u16],
    insns: u64,
    seed: u64,
    simd: bool,
    cfg: &UopConfig,
) -> Result<u64, SweepError> {
    let mut want = core_with_code(program);
    fill_state(&mut want, &mut Rng::new(seed), simd);
    let fault = want.run(insns);
    for split in 0..=insns {
        let mut got = core_with_code(program);
        fill_state(&mut got, &mut Rng::new(seed), simd);
        got.enable_uop(cfg.clone(), &[(CODE_PC, CODE_PC + program.len() as u32)])
            .map_err(SweepError::Engine)?;
        let f = got.run_uop(split).or_else(|| got.run_uop(insns - split));
        let diff = if f != fault {
            Some(ExactDiff {
                field: "fault",
                a: format!("{fault:?}"),
                b: format!("{f:?}"),
            })
        } else {
            exact_diff(&want, &got)
        };
        if let Some(diff) = diff {
            return Err(SweepError::Diff { split, diff });
        }
    }
    Ok(insns + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::gen::random_program;

    #[test]
    fn every_split_of_a_random_program_equals_the_uninterrupted_interpreter_run() {
        let mut rng = Rng::new(31);
        let program = random_program(&mut rng, 10);
        assert_eq!(
            exit_sweep(&program, 10, 5, true, &UopConfig::default()),
            Ok(11)
        );
        assert_eq!(
            exit_sweep(&program, 10, 6, false, &UopConfig::default()),
            Ok(11)
        );
    }

    #[test]
    fn an_unarmable_program_is_an_engine_error() {
        assert!(matches!(
            exit_sweep(&[], 0, 1, false, &UopConfig::default()),
            Err(SweepError::Engine(UopError::OutOfRange { .. }))
        ));
    }
}
