//! Differential fuzz: random programs on random states, uop engine against `Core::run`.
//!
//! OWNER: W3-FUZZ (per-family program shapes and the release iteration counts); the helper
//! [`diff_program`], "an op through lowering versus the interpreter", is complete and is what the
//! families use in their own unit tests. Design 4.6: iteration counts come from `NORD_UOP_FUZZ_N`
//! with a small default so the debug-profile CI job stays inside its limit (ASSUMED 100 programs
//! here; the release gate G1 runs 200,000 blocks per family).
use super::compare::{exact_diff, ExactDiff};
use super::gen::{core_with_code, fill_state, random_program, Rng, CODE_PC};
use crate::core::uop::config::{UopBuildReport, UopConfig, UopError, UopStats};

/// Programs run when `NORD_UOP_FUZZ_N` is not set.
pub const DEFAULT_N: u32 = 100;

/// Programs to run: `NORD_UOP_FUZZ_N` when it parses, else [`DEFAULT_N`].
pub fn iterations() -> u32 {
    std::env::var("NORD_UOP_FUZZ_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_N)
}

/// What one differential run found and did: the verdict plus the evidence that the engine ran
/// something. Every family test asserts [`DiffRun::exercised`], because a run in which nothing
/// was lowered (or the executor declined every block) is identical to the interpreter by
/// construction and proves nothing (the skeleton passes every such test vacuously).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRun {
    /// `None` means identical results (including the fault, if any).
    pub diff: Option<ExactDiff>,
    /// What `enable_uop` built; `insns - interp_insns` instructions went to fast families.
    pub build: UopBuildReport,
    /// Engine counters after the run.
    pub stats: UopStats,
}

impl DiffRun {
    /// Whether fast uops were built and the engine ran steps beyond the `Interp` uops.
    pub fn exercised(&self) -> bool {
        self.build.insns > self.build.interp_insns
            && self.stats.steps_uop > self.stats.steps_interp_uop
    }
}

/// Runs `program` for `insns` steps on two cores with the same state drawn from `seed`: the
/// interpreter on one, the armed uop engine on the other.
pub fn diff_program_run(
    program: &[u16],
    insns: u64,
    seed: u64,
    simd: bool,
    cfg: &UopConfig,
) -> Result<DiffRun, UopError> {
    let (mut a, mut b) = (core_with_code(program), core_with_code(program));
    fill_state(&mut a, &mut Rng::new(seed), simd);
    fill_state(&mut b, &mut Rng::new(seed), simd);
    let build = b.enable_uop(cfg.clone(), &[(CODE_PC, CODE_PC + program.len() as u32)])?;
    let (fa, fb) = (a.run(insns), b.run_uop(insns));
    let diff = if fa != fb {
        Some(ExactDiff {
            field: "fault",
            a: format!("{fa:?}"),
            b: format!("{fb:?}"),
        })
    } else {
        exact_diff(&a, &b)
    };
    Ok(DiffRun {
        diff,
        build,
        stats: b.uop_stats(),
    })
}

/// [`diff_program_run`] without the evidence: `None` means identical results.
pub fn diff_program(
    program: &[u16],
    insns: u64,
    seed: u64,
    simd: bool,
    cfg: &UopConfig,
) -> Result<Option<ExactDiff>, UopError> {
    diff_program_run(program, insns, seed, simd, cfg).map(|r| r.diff)
}

/// Outcome of [`fuzz`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzReport {
    /// Programs run.
    pub programs: u32,
    /// Instructions run in total.
    pub insns: u64,
    /// The seed and difference of the first failing program, if any.
    pub failure: Option<(u64, ExactDiff)>,
}

/// Runs `n` programs of 1 to 12 instructions from the pool of `gen.rs`, stopping at the first
/// difference. A failing `case seed` reproduces through [`diff_program`].
pub fn fuzz(seed: u64, n: u32, cfg: &UopConfig) -> Result<FuzzReport, UopError> {
    let mut rng = Rng::new(seed);
    let mut report = FuzzReport {
        programs: 0,
        insns: 0,
        failure: None,
    };
    for _ in 0..n {
        let case = rng.next_u64();
        let mut r = Rng::new(case);
        let k = 1 + r.below(12);
        let (program, simd) = (random_program(&mut r, k as usize), r.below(2) == 1);
        report.programs += 1;
        report.insns += u64::from(k);
        if let Some(d) = diff_program(&program, u64::from(k), case, simd, cfg)? {
            report.failure = Some((case, d));
            break;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_iteration_count_applies_without_the_environment_variable() {
        if std::env::var_os("NORD_UOP_FUZZ_N").is_none() {
            assert_eq!(iterations(), DEFAULT_N);
        }
    }

    #[test]
    fn a_nop_program_is_identical_on_both_engines_and_an_empty_one_is_rejected() {
        let cfg = UopConfig::default();
        assert_eq!(diff_program(&[0x0001; 5], 5, 9, false, &cfg), Ok(None));
        assert_eq!(
            diff_program(&[], 0, 9, false, &cfg),
            Err(UopError::OutOfRange {
                lo: CODE_PC,
                hi: CODE_PC
            })
        );
    }

    #[test]
    fn a_run_that_lowered_nothing_is_not_counted_as_exercised() {
        let r = diff_program_run(&[0x0001; 5], 5, 9, false, &UopConfig::default()).unwrap();
        assert_eq!(r.diff, None);
        assert!(
            !r.exercised(),
            "the skeleton lowers nothing, so a family test that only checks `diff` is vacuous: {r:?}"
        );
    }

    #[test]
    fn the_fuzz_finds_no_difference_and_a_seed_reproduces_the_report() {
        let cfg = UopConfig::default();
        let n = iterations().min(60);
        let a = fuzz(0xC0FFEE, n, &cfg).unwrap();
        assert_eq!((a.programs, a.failure.clone()), (n, None));
        assert!(a.insns >= u64::from(n));
        assert_eq!(a, fuzz(0xC0FFEE, n, &cfg).unwrap());
    }
}
