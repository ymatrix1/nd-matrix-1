//! The evidence one scenario run leaves behind, and the verdict on a fault mismatch.
//!
//! OWNER: W3-FUZZ. Split out of `scenario.rs` (design P11) so that file stays under the size rule.
use crate::core::uop::verify::compare::ExactDiff;
use crate::core::uop::verify::fuzz::DiffRun;

/// What a scenario did: the verdict and evidence of [`DiffRun`], plus the interrupts the engine
/// core took (equal to the interpreter's when `diff` is `None`) and what the async rounds did.
#[derive(Clone, Debug)]
pub(super) struct Ran {
    pub run: DiffRun,
    pub taken: u64,
    /// Blocks the async rounds installed, and the fast steps run after the first such install.
    pub installed: u64,
    pub fast_after_install: u64,
}

impl Ran {
    /// Whether the case ran fast uops (the rule of `DiffRun::exercised`) and entered blocks.
    pub fn exercised(&self) -> bool {
        self.run.exercised() && self.run.stats.blocks_run > 0
    }
}

/// A difference of the two cores' faults (`None` when they are equal).
pub(super) fn fault_diff<E: std::fmt::Debug + PartialEq>(
    a: Option<E>,
    b: Option<E>,
) -> Option<ExactDiff> {
    (a != b).then(|| ExactDiff {
        field: "fault",
        a: format!("{a:?}"),
        b: format!("{b:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_faults_are_no_difference_and_unequal_ones_name_the_fault_field() {
        assert_eq!(fault_diff(Some(1), Some(1)), None);
        assert_eq!(fault_diff::<u8>(None, None), None);
        let d = fault_diff(None, Some(3)).expect("a difference");
        assert_eq!(
            (d.field, d.a.as_str(), d.b.as_str()),
            ("fault", "None", "Some(3)")
        );
    }

    #[test]
    fn a_run_without_a_fast_uop_is_not_exercised() {
        let ran = Ran {
            run: DiffRun {
                diff: None,
                build: Default::default(),
                stats: Default::default(),
            },
            taken: 0,
            installed: 0,
            fast_after_install: 0,
        };
        assert!(!ran.exercised());
    }
}
