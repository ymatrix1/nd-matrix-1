//! OWNER: W2-PEEP. Predicate grouping pass (skeleton: no-op).
//!
//! Spec (design note 3.1, WP W2-PEEP): merge runs of 2..=8 uops that sit under the same
//! condition (each wrapped by its own `PRED` from `decode::wrap_pred`) under one `PRED` whose
//! `imm` is the combined skip count. The condition of a run must not be changed by a uop inside
//! it (flag-setting uops end the run). Never changes the `done` or `last_pc` of an exit; honours
//! `opts.predicate`. Runs last, because it is the only pass that depends on uop positions.
//!
//! Registered statically in `trace::build` (order: `fuse`, `flagopt`, `pred`).

use super::trace::BuildOpts;
use super::uop::Uop;

/// Group predicates of `uops` in place.
pub fn run(uops: &mut Vec<Uop>, opts: &BuildOpts) {
    let _ = (uops, opts); // skeleton: the work package fills this in
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::uop::k;

    #[test]
    fn the_skeleton_leaves_the_trace_untouched() {
        let mut uops = vec![Uop::new(k::PRED, 0x1000), Uop::new(k::ADD_IMM, 0x1000)];
        let before = uops.clone();
        run(&mut uops, &BuildOpts::default());
        assert_eq!(uops, before);
    }
}
