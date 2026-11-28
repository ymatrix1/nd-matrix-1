//! The exit sweep on scenarios: run the engine for `k` steps, then for the rest, for every `k`,
//! and compare with one uninterrupted interpreter run.
//!
//! OWNER: W3-FUZZ. `verify::sweep::exit_sweep` does the same for straight programs on a fixed
//! state fill; this one takes any [`Scenario`] (control flow, loops, a planted wrong handler,
//! interrupts) so the exit protocol is exercised at every uop boundary of every shape: a block cut
//! by the step limit materializes the interpreter state at that boundary and the next entry
//! resumes in the middle of a block through the pc map (design 6.5).
use super::scenario::{plant, Scenario};
use crate::core::uop::config::UopError;
use crate::core::uop::verify::compare::{exact_diff, ExactDiff};
use crate::core::uop::verify::fuzz::DiffRun;

/// A sweep that found a split at which the engine's result differs.
#[derive(Clone, Debug)]
pub(super) struct SplitDiff {
    pub split: u64,
    pub diff: ExactDiff,
}

/// Result of a sweep: the splits compared and the fast steps the engine ran over all of them.
#[derive(Clone, Debug)]
pub(super) struct Swept {
    pub splits: u64,
    pub fast_steps: u64,
}

/// Sweeps every split `0..=steps` of the first `Step::Run` of `s` (the scenario's plan must be a
/// single run, interrupts allowed). The first difference is returned with its split.
pub(super) fn split_sweep(s: &Scenario, steps: u64) -> Result<Result<Swept, SplitDiff>, UopError> {
    let mut want = s.core();
    let fault = want.run(steps);
    let mut fast_steps = 0;
    for split in 0..=steps {
        let mut got = s.core();
        let build = got.enable_uop(s.cfg.clone(), &s.ranges())?;
        if let Some(m) = s.mutant {
            plant(&mut got, m, s.nth);
        }
        let f = got.run_uop(split).or_else(|| got.run_uop(steps - split));
        let st = got.uop_stats();
        fast_steps += st.steps_uop.saturating_sub(st.steps_interp_uop);
        let diff = if f != fault {
            Some(ExactDiff {
                field: "fault",
                a: format!("{fault:?}"),
                b: format!("{f:?}"),
            })
        } else {
            exact_diff(&want, &got)
        };
        let _ = DiffRun {
            diff: None,
            build,
            stats: st,
        };
        if let Some(diff) = diff {
            return Ok(Err(SplitDiff { split, diff }));
        }
    }
    Ok(Ok(Swept {
        splits: steps + 1,
        fast_steps,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::fuzz::shapes::{straight, ALL};
    use crate::core::uop::fuzz::state::Prep;
    use crate::core::uop::verify::gen::Rng;
    use crate::core::uop::verify::mutate::Mutant;

    #[test]
    fn every_split_of_a_mixed_program_equals_the_uninterrupted_run_and_ran_fast() {
        let p = straight(&mut Rng::new(4), &ALL, 8);
        let s = Scenario::new("split", 4, p, 8);
        let r = split_sweep(&s, 8).unwrap().unwrap();
        assert_eq!(r.splits, 9);
        assert!(r.fast_steps > 0, "{r:?}");
    }

    #[test]
    fn a_planted_wrong_handler_is_found_at_a_split() {
        // some programs overwrite what the mutant changed: one of the first few shows it
        let found = (4..24)
            .find_map(|seed| {
                let p = straight(&mut Rng::new(seed), &ALL, 8);
                let mut s = Scenario::new("split", seed, p, 8);
                s.prep = Prep::full(false);
                s.mutant = Some(Mutant::ResultBit);
                split_sweep(&s, 8).unwrap().err()
            })
            .expect("a split that shows the planted wrong handler");
        assert_eq!(found.diff.field, "r", "{found:?}");
    }

    #[test]
    fn an_unarmable_scenario_is_an_error() {
        let s = Scenario::new("empty", 1, Vec::new(), 1);
        assert!(split_sweep(&s, 1).is_err());
    }
}
