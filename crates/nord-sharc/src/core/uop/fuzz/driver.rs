//! The loop around a case generator: draws the cases of one test, runs each, collects every
//! divergence and every vacuous case, and fails once at the end with all of them.
//!
//! OWNER: W3-FUZZ. A test that stops at the first divergence hides the second; the long run of
//! a fuzz is for finding all of them, so every failing case is reported with its seed, its
//! program and the first differing field (`Scenario::describe`), up to [`SHOWN`] of them in full.
use super::knobs::{cases, rng_for, seed};
use super::scenario::{run, Scenario};
use crate::core::uop::verify::gen::Rng;

/// Failing cases printed in full; the rest are only counted.
const SHOWN: usize = 6;

/// What a sweep ran: summed engine counters of every case, for the assertions that look at them.
#[derive(Clone, Debug, Default)]
pub(super) struct Totals {
    pub cases: u64,
    pub steps_uop: u64,
    pub steps_interp_uop: u64,
    pub steps_core_step: u64,
    pub blocks_run: u64,
    pub taken: u64,
    pub smc: u64,
    pub maintain: u64,
    /// Blocks installed by async rounds, and fast steps run after the first install of a case.
    pub installed: u64,
    pub fast_after_install: u64,
    pub exits: [u64; 6],
    /// Cases flagged `heavy` and how many of them ran a fast uop.
    pub heavy: (u64, u64),
}

/// One vacuous case in this many is tolerated per sweep. ASSUMED: measured 1.5 percent at 400
/// cases per sweep (a slice too short for its first block); 5 percent leaves room for a seed.
const VACUOUS_ONE_IN: u64 = 20;

/// Runs `default_cases` cases (or `NS4_FUZZ_CASES`) drawn by `gen(rng, case_seed)`; `id` names the
/// stream of the test. Panics at the end if any case diverged, was vacuous or could not arm the
/// engine.
#[track_caller]
pub(super) fn sweep(
    name: &str,
    id: u64,
    default_cases: u64,
    mut gen: impl FnMut(&mut Rng, u64) -> Scenario,
) -> Totals {
    let mut rng = rng_for(id);
    let mut totals = Totals::default();
    let mut failures: Vec<String> = Vec::new();
    let mut vacuous: Vec<String> = Vec::new();
    for n in 0..cases(default_cases) {
        let case_seed = rng.next_u64();
        let s = gen(&mut Rng::new(case_seed), case_seed);
        totals.cases += 1;
        match run(&s) {
            Err(e) => failures.push(format!(
                "case {n}: cannot arm the engine: {e}\n{}",
                s.describe()
            )),
            Ok(r) => {
                let st = r.run.stats;
                totals.steps_uop += st.steps_uop;
                totals.steps_interp_uop += st.steps_interp_uop;
                totals.steps_core_step += st.steps_core_step;
                totals.blocks_run += st.blocks_run;
                totals.taken += r.taken;
                totals.smc += st.smc;
                totals.maintain += st.maintain_calls;
                totals.installed += r.installed;
                totals.fast_after_install += r.fast_after_install;
                for (t, e) in totals.exits.iter_mut().zip(st.exits) {
                    *t += e;
                }
                if let Some(d) = &r.run.diff {
                    failures.push(format!(
                        "case {n}: DIVERGENCE first field `{}`: {d}\n{}",
                        d.field,
                        s.describe()
                    ));
                } else if s.heavy {
                    totals.heavy.0 += 1;
                    totals.heavy.1 += u64::from(r.exercised());
                } else if !r.exercised() {
                    vacuous.push(format!(
                        "case {n}: vacuous, no fast uop ran: {:?} {st:?}\n{}",
                        r.run.build,
                        s.describe()
                    ));
                }
            }
        }
    }
    // A random case can end before its first fast uop: a slice of a few steps whose block does not
    // fit, an exception at the first instruction. Such a case proves nothing, so more than one in
    // twenty of them fails the sweep (and at the CI bound a single one is allowed to pass).
    if vacuous.len() as u64 * VACUOUS_ONE_IN > totals.cases.max(VACUOUS_ONE_IN) {
        failures.append(&mut vacuous);
    }
    let (heavy, heavy_run) = totals.heavy;
    if heavy_run * 2 < heavy {
        failures.push(format!(
            "only {heavy_run} of {heavy} exception-heavy cases ran a fast uop (half are required)"
        ));
    }
    if !failures.is_empty() {
        let shown = failures
            .iter()
            .take(SHOWN)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "{name}: {} of {} cases failed (NS4_FUZZ_SEED={}, a case reproduces from its seed):\n{shown}",
            failures.len(),
            totals.cases,
            seed()
        );
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::fuzz::shapes::straight;
    use crate::core::uop::fuzz::sieve::Fam;

    fn adds(rng: &mut Rng, case: u64) -> Scenario {
        let p = straight(rng, &[Fam::Float], 4);
        Scenario::new("driver-test", case, p, 4)
    }

    #[test]
    fn a_sweep_runs_the_requested_cases_and_sums_the_counters() {
        let t = sweep("t", 90, 5, adds);
        let n = cases(5);
        assert_eq!(t.cases, n);
        assert!(t.steps_uop >= n && t.blocks_run >= n, "{t:?}");
    }

    #[test]
    #[should_panic(expected = "vacuous")]
    fn a_vacuous_case_fails_the_sweep() {
        sweep("t", 91, 2, |_, case| {
            Scenario::new("nops", case, vec![0x0001; 3], 3)
        });
    }

    #[test]
    #[should_panic(expected = "cannot arm the engine")]
    fn a_case_the_engine_cannot_be_armed_on_is_reported() {
        sweep("t", 92, 1, |_, case| {
            Scenario::new("empty", case, Vec::new(), 1)
        });
    }
}
