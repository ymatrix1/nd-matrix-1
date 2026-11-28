//! OWNER: W3-VERIF. Layer L5, mutation testing of the lowering and the executors (skeleton: the
//! switch and the report, no mutants yet).
//!
//! Spec (design note 8, layer L5, WP W3-VERIF): at least 40 mutants (flip a carry rule, drop a
//! rotate, swap write-back order, wrong condition polarity, off-by-one `done`, wrong `last_pc`).
//! A mutant is a `cfg(test)` branch inside a lowerer or an executor, `if mutants::is_active(N)
//! { wrong } else { right }`, enabled for the current thread only through [`with`]. [`run_all`]
//! runs the L0 and L1 suites once per mutant at the default case count and records which ones
//! failed; a mutant that survives is a hole in the oracle. The kill rate must be at least 95 %,
//! and the comparator itself has mutants (a comparator that ignores a field must be caught).
//!
//! Ids are reserved per work package so the sites can be written in parallel without a clash:
//! `0..=99` W1-DP, `100..=199` W1-MEM, `200..=299` W1-FLOW, `300..=399` W1-XALU and W1-XMEM,
//! `400..=499` W2-XCORE, `500..=599` W2-PEEP, `600..=699` W2-DRV and W1-CACHE, `700..=799` the
//! comparator, `800..=899` the native lowerers. A site uses `is_active(<its id>)`; W3-VERIF
//! registers the `(id, name)` pairs in [`MUTANTS`] after the lowerers and executors land (an edit
//! of this file owned by W3-VERIF) and checks that every registered id has a site.

#![allow(dead_code)]

use std::cell::Cell;

thread_local! {
    static ACTIVE: Cell<Option<u32>> = const { Cell::new(None) };
}

/// `(id, name)` of every known mutant, ids drawn from the per-package blocks above. The skeleton
/// has none.
pub(crate) const MUTANTS: &[(u32, &str)] = &[];

/// The mutant enabled on this thread, if any.
pub(crate) fn active() -> Option<u32> {
    ACTIVE.with(Cell::get)
}

/// Whether mutant `id` is enabled on this thread (the guard of a mutant site).
pub(crate) fn is_active(id: u32) -> bool {
    active() == Some(id)
}

/// Run `f` with mutant `id` enabled on this thread; the previous state is restored even when `f`
/// panics (a failing oracle panics, which is how a mutant is killed).
pub(crate) fn with<R>(id: u32, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<u32>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.set(self.0));
        }
    }
    let _restore = Restore(ACTIVE.with(|a| a.replace(Some(id))));
    f()
}

/// Outcome of a mutation run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub total: usize,
    pub killed: usize,
    /// Names of the mutants no test failed on.
    pub survivors: Vec<&'static str>,
}

impl Report {
    /// Killed fraction in `0.0..=1.0` (1.0 for an empty report: nothing survived).
    pub fn kill_rate(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.killed as f64 / self.total as f64
        }
    }
}

/// Run `oracle` (the L0 and L1 suites, returning `true` when it passes) once per mutant. The
/// skeleton has no mutants, so the report is empty.
pub(crate) fn run_all(oracle: impl Fn() -> bool + std::panic::RefUnwindSafe) -> Report {
    let mut report = Report::default();
    for &(id, name) in MUTANTS {
        report.total += 1;
        let passed = std::panic::catch_unwind(|| with(id, &oracle)).unwrap_or(false);
        if passed {
            report.survivors.push(name);
        } else {
            report.killed += 1;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mutant_is_active_only_inside_with_and_the_state_is_restored() {
        assert_eq!(active(), None);
        with(3, || {
            assert!(is_active(3) && !is_active(4));
            with(4, || assert!(is_active(4) && !is_active(3)));
            assert!(is_active(3), "the outer mutant is back");
        });
        assert_eq!(active(), None);
    }

    #[test]
    fn the_state_is_restored_when_the_oracle_panics() {
        let caught = std::panic::catch_unwind(|| with(7, || panic!("killed")));
        assert!(caught.is_err());
        assert_eq!(active(), None);
    }

    #[test]
    fn the_switch_is_per_thread() {
        with(1, || {
            assert!(std::thread::spawn(active).join().unwrap().is_none());
            assert!(is_active(1));
        });
    }

    #[test]
    fn kill_rate_counts_survivors_and_an_empty_report_is_clean() {
        assert_eq!(Report::default().kill_rate(), 1.0);
        let r = Report {
            total: 40,
            killed: 38,
            survivors: vec!["a", "b"],
        };
        assert!((r.kill_rate() - 0.95).abs() < 1e-12);
    }

    #[test]
    fn registered_ids_are_unique() {
        let mut ids: Vec<u32> = MUTANTS.iter().map(|m| m.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MUTANTS.len());
    }

    #[test]
    fn the_skeleton_has_no_mutants_so_run_all_reports_nothing_to_kill() {
        assert_eq!(MUTANTS.len(), 0, "the skeleton registers no mutant");
        let r = run_all(|| true);
        assert_eq!((r.total, r.killed, r.kill_rate()), (0, 0, 1.0));
    }
}
