//! Many cases through the rig of `diff.rs`: the two handler tables, the second run of every
//! bailing case, and the per-kind counters the acceptance tests assert on.
//!
//! OWNER: W2-FIXED, test support.
use super::diff::{run_case_with, Case, Mode};
use crate::core::uop::hot::UopHot;
use crate::core::uop::types::{Exit, Handler};
use crate::core::Core;
use std::collections::BTreeMap;

/// What a sweep covered, per kind: cases that ran the handler to `Ok`, that bailed, that changed
/// the data, that ran PEy, where the interpreter raised an exception, and bailing cases re-run
/// through the handler without the bail; and how many programs the family declined.
#[derive(Default, Debug)]
pub(super) struct Tally {
    pub declined: usize,
    /// kind -> `[ok, slow, changed, pey_ran, raised, forced]`
    pub kinds: BTreeMap<u16, [usize; 6]>,
}

impl Tally {
    /// The counters of `kind` (all zero when it never occurred).
    pub fn of(&self, kind: u16) -> [usize; 6] {
        self.kinds.get(&kind).copied().unwrap_or_default()
    }

    /// Cases that ran the handler to completion, over every kind.
    pub fn ok(&self) -> usize {
        self.kinds.values().map(|c| c[0]).sum()
    }

    /// Cases the handler bailed on, over every kind.
    pub fn slow(&self) -> usize {
        self.kinds.values().map(|c| c[1]).sum()
    }
}

/// The two handler tables of the rig: the registered handlers and the ones without the bail.
pub(super) struct Tables {
    pub real: Vec<Handler>,
    pub forced: Vec<Handler>,
}

impl Tables {
    pub fn new() -> Tables {
        Tables {
            real: UopHot::handler_table().into_vec(),
            forced: super::forced_table(),
        }
    }

    /// [`run_case_with`] through the registered handler and, when it bails, a second time through
    /// the handler without the bail: that proves the arithmetic of the exception cases (the
    /// saturated and wrapped values, the flags, the spills) that the real handler leaves to the
    /// interpreter. A case the real handler ran is the same code path in both, so it is run once.
    /// Returns the case of the registered handler and whether the second run happened (and agreed
    /// with the interpreter, `Err` otherwise).
    pub fn run(
        &self,
        program: &[u16],
        seed: u64,
        simd: bool,
        setup: &dyn Fn(&mut Core),
    ) -> Result<Option<(Case, bool)>, String> {
        let Some(case) = run_case_with(&self.real, Mode::Real, program, seed, simd, setup)? else {
            return Ok(None);
        };
        if case.exit != Exit::Slow {
            return Ok(Some((case, false)));
        }
        match run_case_with(&self.forced, Mode::Forced, program, seed, simd, setup)? {
            Some(f)
                if f.exit == Exit::Ok && f.raised && f.after.sys.stky == case.after.sys.stky =>
            {
                Ok(Some((case, true)))
            }
            _ => Err(format!(
                "program {program:04x?} seed {seed}: the forced rerun differs"
            )),
        }
    }
}

/// Runs every program on `seeds` states in both PEy modes through [`Tables::run`], panics with the
/// first mismatch, calls `observe` on every agreeing case and returns the tally.
pub(super) fn sweep(programs: &[Vec<u16>], seeds: u64, observe: &mut dyn FnMut(&Case)) -> Tally {
    let tables = Tables::new();
    let mut t = Tally::default();
    for (i, p) in programs.iter().enumerate() {
        for simd in [false, true] {
            for s in 0..seeds {
                let seed = 1 + s + 1000 * i as u64 + u64::from(simd);
                let (case, forced) = match tables.run(p, seed, simd, &|_| ()) {
                    Err(text) => panic!("{text}"),
                    Ok(None) => {
                        t.declined += 1;
                        continue;
                    }
                    Ok(Some(done)) => done,
                };
                let e = t.kinds.entry(case.kind).or_default();
                e[usize::from(case.exit == Exit::Slow)] += 1;
                e[2] += usize::from(case.changed());
                e[3] += usize::from(case.ran.1);
                e[4] += usize::from(case.raised);
                e[5] += usize::from(forced);
                observe(&case);
            }
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::exec::h_fixed::fixture::random_programs;
    use crate::core::uop::verify::gen::Rng;

    #[test]
    fn a_sweep_counts_what_ran_and_runs_every_bail_a_second_time() {
        // R? = R? + R?: the first program of an opcode is `IF TRUE`; 6 programs, 2 modes, 8 seeds
        let programs = random_programs(0, &[0x01], 6, &mut Rng::new(3));
        let mut seen = 0;
        let t = sweep(&programs, 8, &mut |_| seen += 1);
        let [ok, slow, changed, pey, raised, forced] = t.of(0x201);
        assert_eq!((ok + slow, seen, t.declined), (96, 96, 0));
        assert_eq!((t.ok(), t.slow()), (ok, slow));
        assert!(
            slow > 0 && slow == raised && forced == slow,
            "{:?}",
            t.of(0x201)
        );
        assert!(changed > 40 && pey > 10, "{:?}", t.of(0x201));
        assert_eq!(
            t.of(0x202),
            [0; 6],
            "a kind that did not occur counts nothing"
        );
        // a word the family declines is counted as declined, not run
        let t = sweep(&[vec![0x0001]], 3, &mut |_| unreachable!());
        assert_eq!((t.declined, t.ok()), (6, 0));
    }

    #[test]
    fn the_second_run_of_a_bail_is_the_handler_without_it() {
        // MAX + 1 on PEx: the registered handler bails, the other one runs and wraps or saturates
        let tables = Tables::new();
        let setup = |c: &mut Core| (c.r[1], c.r[2]) = (0x7FFF_FFFF, 1);
        let prog = crate::core::uop::exec::h_fixed::fixture::t2a(31, 0x1312);
        let (case, forced) = tables.run(&prog, 4, false, &setup).unwrap().unwrap();
        assert_eq!((case.exit, forced), (Exit::Slow, true));
        let (ok, forced) = tables
            .run(&prog, 4, false, &|c| (c.r[1], c.r[2]) = (1, 2))
            .unwrap()
            .unwrap();
        assert_eq!(
            (ok.exit, forced),
            (Exit::Ok, false),
            "no bail, no second run"
        );
        assert_eq!(tables.real.len(), tables.forced.len());
    }
}
