//! OWNER: W2-PEEP. Dead-flag elimination pass (skeleton: no-op).
//!
//! Spec (design note 3.1, WP W2-PEEP): turn an S-flag definition (`k::dp(form, op, true)`, `CMP`,
//! `TST`) that is overwritten before any reader into its non-S twin. A flag reader is a
//! conditional uop, a `GUARD`, a `PRED`, an `adc`/`sbc`/`rsc` or a shifter carry use; a store, a
//! guard, a `GENERIC*`, an `MRS` and every exit make all four flags live (state must be exact
//! there, I4). Never changes the `done` or `last_pc` of an exit; honours `opts.dead_flags`.
//!
//! Registered statically in `trace::build` (order: `fuse`, `flagopt`, `pred`).

use super::trace::BuildOpts;
use super::uop::Uop;

/// Drop dead flag definitions of `uops` in place.
pub fn run(uops: &mut Vec<Uop>, opts: &BuildOpts) {
    let _ = (uops, opts); // skeleton: the work package fills this in
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::uop::k;

    #[test]
    fn the_skeleton_leaves_the_trace_untouched() {
        let mut uops = vec![Uop::new(k::CMP_IMM, 0x1000), Uop::new(k::MOV_IMM, 0x1004)];
        let before = uops.clone();
        run(&mut uops, &BuildOpts::default());
        assert_eq!(uops, before);
    }
}
