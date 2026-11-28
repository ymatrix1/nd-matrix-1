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
//! comparator, `800..=899` the native lowerers, `900..=949` W2-MUL (`lower_mul.rs`; the dispatch site in
//! `exec_alu.rs` is 317 of the W1-XALU block). A site uses `is_active(<its id>)`; W3-VERIF
//! registers the `(id, name)` pairs in [`MUTANTS`] after the lowerers and executors land (an edit
//! of this file owned by W3-VERIF) and checks that every registered id has a site.

#![allow(dead_code)]

use std::cell::Cell;

thread_local! {
    static ACTIVE: Cell<Option<u32>> = const { Cell::new(None) };
}

/// `(id, name)` of every known mutant, ids drawn from the per-package blocks above. The native
/// memory and flow lowerers (W2-NMEM, `850..=893`) are registered here; their sites exist only
/// with the `sb-native` feature, and `native/lower_mem/tests.rs` checks that this list and the
/// modules' own `MUTANTS` agree and kills each of them.
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    #[cfg(feature = "sb-native")]
    (850, "native mem: a load ignores HOOK_R"),
    #[cfg(feature = "sb-native")]
    (851, "native mem: a store ignores HOOK_C and HOOK_W"),
    #[cfg(feature = "sb-native")]
    (852, "native mem: an unaligned word load is not rotated"),
    #[cfg(feature = "sb-native")]
    (853, "native mem: a word load uses the unaligned address"),
    #[cfg(feature = "sb-native")]
    (854, "native mem: a halfword access uses the odd address"),
    #[cfg(feature = "sb-native")]
    (855, "native mem: ldrsb zero-extends"),
    #[cfg(feature = "sb-native")]
    (856, "native mem: ldrsh zero-extends"),
    #[cfg(feature = "sb-native")]
    (857, "native mem: the write-back is skipped"),
    #[cfg(feature = "sb-native")]
    (858, "native mem: the U flag is inverted"),
    #[cfg(feature = "sb-native")]
    (
        859,
        "native mem: the P flag is ignored (always pre-indexed)",
    ),
    #[cfg(feature = "sb-native")]
    (
        860,
        "native mem: a load with rn == rd lets the write-back win",
    ),
    #[cfg(feature = "sb-native")]
    (861, "native mem: a store reads rd after the write-back"),
    #[cfg(feature = "sb-native")]
    (862, "native mem: ldrd swaps the halves"),
    #[cfg(feature = "sb-native")]
    (863, "native mem: ldrd/strd skip the page-end check"),
    #[cfg(feature = "sb-native")]
    (864, "native mem: ldm/stm skip the page-end check"),
    #[cfg(feature = "sb-native")]
    (
        865,
        "native mem: stm writes the base back before the first store",
    ),
    #[cfg(feature = "sb-native")]
    (
        866,
        "native mem: ldm writes the base back although it is in the list",
    ),
    #[cfg(feature = "sb-native")]
    (
        867,
        "native mem: ldm/stm decrement-before starts one word too low",
    ),
    #[cfg(feature = "sb-native")]
    (868, "native mem: a register offset ignores the shift type"),
    #[cfg(feature = "sb-native")]
    (
        869,
        "native mem: a register offset LSR #0 shifts by 0, not 32",
    ),
    #[cfg(feature = "sb-native")]
    (
        870,
        "native mem: a register offset ASR #0 shifts by 0, not 32",
    ),
    #[cfg(feature = "sb-native")]
    (871, "native mem: ldr_abs does not rotate"),
    #[cfg(feature = "sb-native")]
    (
        872,
        "native mem: the side exit reports the next instruction",
    ),
    #[cfg(feature = "sb-native")]
    (873, "native mem: the page-end check is off by one word"),
    #[cfg(feature = "sb-native")]
    (874, "native mem: strb stores a halfword"),
    #[cfg(feature = "sb-native")]
    (875, "native mem: strh stores a byte"),
    #[cfg(feature = "sb-native")]
    (877, "native mem: ldm loads descending"),
    #[cfg(feature = "sb-native")]
    (
        878,
        "native mem: a register offset is added whatever U says",
    ),
    #[cfg(feature = "sb-native")]
    (879, "native mem: a halfword register offset is shifted"),
    #[cfg(feature = "sb-native")]
    (
        880,
        "native flow: a guard takes its exit when the condition fails",
    ),
    #[cfg(feature = "sb-native")]
    (
        881,
        "native flow: guard_reg_eq leaves when the register is equal",
    ),
    #[cfg(feature = "sb-native")]
    (
        882,
        "native flow: a predicated body runs when the condition fails",
    ),
    #[cfg(feature = "sb-native")]
    (883, "native flow: HI is C or Z"),
    #[cfg(feature = "sb-native")]
    (884, "native flow: GE is N != V"),
    #[cfg(feature = "sb-native")]
    (
        885,
        "native flow: loop_back leaves for the interpreter instead of looping",
    ),
    #[cfg(feature = "sb-native")]
    (
        887,
        "native flow: the second half of a fused pair retires nothing before it",
    ),
    #[cfg(feature = "sb-native")]
    (888, "native flow: LS is not C and Z"),
    #[cfg(feature = "sb-native")]
    (889, "native flow: GT ignores Z"),
    #[cfg(feature = "sb-native")]
    (890, "native flow: LE is Z and N != V"),
    #[cfg(feature = "sb-native")]
    (891, "native flow: CS is not C"),
    #[cfg(feature = "sb-native")]
    (
        892,
        "native flow: guard_reg_eq compares the low halfword only",
    ),
    #[cfg(feature = "sb-native")]
    (893, "native flow: a predicated body is one uop short"),
];

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
    fn run_all_runs_the_oracle_once_per_registered_mutant() {
        let never_fails = run_all(|| true);
        assert_eq!(
            (
                never_fails.total,
                never_fails.killed,
                never_fails.survivors.len()
            ),
            (MUTANTS.len(), 0, MUTANTS.len()),
            "an oracle that never fails kills nothing"
        );
        let always_fails = run_all(|| false);
        assert_eq!(
            (always_fails.total, always_fails.killed),
            (MUTANTS.len(), MUTANTS.len())
        );
        assert_eq!(always_fails.kill_rate(), 1.0);
    }
}
