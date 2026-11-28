//! OWNER: W2-PEEP. Fusion pass (skeleton: no-op).
//!
//! Spec (design note 3.1, WP W2-PEEP): rewrite `CMP`/`TST` followed by the `GUARD` of the `Bcc`
//! it feeds into `CMP_GUARD_I`/`CMP_GUARD_R`/`TST_GUARD_I`, adjacent `MOV_IMM` pairs into
//! `MOV_PAIR` and adjacent `LDR_IMM` pairs into `LDR_PAIR` (the top bigrams of a G0 counter).
//! Each fused kind is defined as its unfused expansion; the pass never changes the `done` or
//! `last_pc` of an exit, never merges across a `PRED` boundary and honours `opts.fuse`.
//!
//! Registered statically in `trace::build` (order: `fuse`, `flagopt`, `pred`).

use super::trace::BuildOpts;
use super::uop::Uop;

/// Fuse `uops` in place.
pub fn run(uops: &mut Vec<Uop>, opts: &BuildOpts) {
    let _ = (uops, opts); // skeleton: the work package fills this in
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::uop::k;

    #[test]
    fn the_skeleton_leaves_the_trace_untouched() {
        let mut uops = vec![Uop::new(k::CMP_IMM, 0x1000), Uop::new(k::GUARD, 0x1004)];
        let before = uops.clone();
        run(&mut uops, &BuildOpts::default());
        assert_eq!(uops, before);
    }
}
