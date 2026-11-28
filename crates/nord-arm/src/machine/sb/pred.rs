//! OWNER: W2-PEEP. Predicate grouping pass (design note 3.1, WP W2-PEEP).
//!
//! `decode::wrap_pred` puts every conditional instruction under its own `PRED` (`cond`, `imm` =
//! uops of the body). Runs of 2..=[`MAX_RUN`] of them that sit back to back under the same
//! condition become one `PRED` whose `imm` is the combined body, and the executor tests the
//! condition once:
//!
//! ```text
//! PRED eq,2 ; a ; b ; PRED eq,1 ; c      ->      PRED eq,3 ; a ; b ; c
//! ```
//!
//! This is exact when the flags cannot change between the members, so a run ends after the first
//! member whose body may write the flags (an S operation, a compare, `GENERIC*`, `MSR_C`:
//! `flagopt::effect::writes_flags`); that member is still the last of its run, because the test
//! at its own start saw the unchanged flags. A member is a `PRED` whose body holds only data
//! processing, memory, `GENERIC` and `MRS`/`MSR_C` uops: no control uop, no fused uop, no nested
//! `PRED`. A failing condition still retires every instruction of the run (static `done`), so
//! no exit changes; the removed `PRED`s carry `n = 0`.
//!
//! Position-dependent state is only the skip count, which is why the pass runs last. Honours
//! `opts.predicate`.
//!
//! ASSUMED: [`MAX_RUN`] = 8 (design note 3.1: "predicate grouping 2..8"); a longer run would
//! only save one more condition test per member.
//!
//! # Mutation sites (L5, ids `560..=589`)
//!
//! [`MUTANTS`] lists them; `fuse/kills.rs` proves each is killed by this package's suites.

use super::flagopt::effect::writes_flags;
use super::fuse::mutant;
use super::trace::BuildOpts;
use super::uop::{holds, k, Uop};

#[cfg(test)]
pub(super) mod cases;
#[cfg(test)]
pub(super) mod diff;

/// Most members of one merged run.
pub const MAX_RUN: usize = 8;

/// `(id, name)` of the mutation sites of the predicate pass (block `560..=589`).
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (560, "pred: members of different conditions are merged"),
    (561, "pred: a body that writes flags does not end the run"),
    (562, "pred: the merged skip count is one too long"),
    (563, "pred: runs longer than MAX_RUN are merged"),
    (564, "pred: members that are not adjacent are merged"),
    (565, "pred: a body with a control uop is merged"),
    (566, "pred: the switch is ignored"),
];

/// What one run of the pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Grouped {
    /// Merged runs.
    pub runs: u32,
    /// `PRED`s removed (members beyond the first of each run).
    pub removed: u32,
}

/// The flags bits (`n << 3 | z << 2 | c << 1 | v`) the condition `cond` depends on, found from
/// [`holds`] itself: a bit is read when flipping it can change the verdict for some nibble.
pub fn cond_reads(cond: u8) -> u8 {
    (0..4u8)
        .map(|b| 1u8 << b)
        .filter(|bit| (0..16u8).any(|f| holds(cond, f) != holds(cond, f ^ bit)))
        .fold(0, |a, bit| a | bit)
}

/// `mask[i]` is true when uop `i` lies in the body of a `PRED` (bodies that run past the end are
/// clamped).
pub fn body_mask(uops: &[Uop]) -> Vec<bool> {
    let mut mask = vec![false; uops.len()];
    for (p, u) in uops.iter().enumerate() {
        if u.kind == k::PRED {
            let end = (p + 1 + u.imm as usize).min(uops.len());
            mask[p + 1..end].fill(true);
        }
    }
    mask
}

/// Group predicates of `uops` in place.
pub fn run(uops: &mut Vec<Uop>, opts: &BuildOpts) {
    run_counted(uops, opts);
}

/// A uop a merged body may hold.
fn mergeable(u: &Uop) -> bool {
    mutant(565)
        || matches!(k::family(u.kind), k::Family::Dp | k::Family::Mem)
        || matches!(u.kind, k::GENERIC | k::MRS | k::MSR_C)
}

/// End (exclusive) of the body of the mergeable `PRED` at `i`, or `None`.
fn member_end(uops: &[Uop], i: usize) -> Option<usize> {
    let p = uops[i];
    let end = i + 1 + p.imm as usize;
    (p.kind == k::PRED && p.imm > 0 && end <= uops.len() && uops[i + 1..end].iter().all(mergeable))
        .then_some(end)
}

/// Whether the body `uops[from..to]` may change the flags.
fn body_writes(uops: &[Uop], from: usize, to: usize) -> bool {
    uops[from..to].iter().any(writes_flags)
}

/// [`run`], reporting what it did.
pub fn run_counted(uops: &mut Vec<Uop>, opts: &BuildOpts) -> Grouped {
    let mut stats = Grouped::default();
    if !opts.predicate && !mutant(566) {
        return stats;
    }
    let mut out = Vec::with_capacity(uops.len());
    let mut i = 0;
    while i < uops.len() {
        let head = uops[i];
        let Some(first_end) = member_end(uops, i) else {
            out.push(head);
            i += 1;
            continue;
        };
        // bodies of the run so far, and whether none of them may change the flags
        let mut bodies = vec![(i + 1, first_end)];
        let mut steady = !body_writes(uops, i + 1, first_end) || mutant(561);
        let mut at = first_end;
        let limit = MAX_RUN + usize::from(mutant(563));
        while bodies.len() < limit && steady {
            let gap = usize::from(mutant(564) && at < uops.len() && uops[at].kind != k::PRED);
            let next = at + gap;
            let same = next < uops.len()
                && uops[next].kind == k::PRED
                && (uops[next].cond == head.cond || mutant(560));
            let Some(end) = member_end(uops, next).filter(|_| same) else {
                break;
            };
            if gap == 1 {
                bodies.push((at, at + 1)); // the stray uop runs under the condition too
            }
            bodies.push((next + 1, end));
            steady = !body_writes(uops, next + 1, end) || mutant(561);
            at = end;
        }
        if bodies.len() < 2 {
            out.push(head);
            i += 1;
            continue;
        }
        let mut merged = head;
        merged.imm =
            bodies.iter().map(|(a, b)| (b - a) as u32).sum::<u32>() + u32::from(mutant(562));
        out.push(merged);
        for &(a, b) in &bodies {
            out.extend_from_slice(&uops[a..b]);
        }
        stats.runs += 1;
        stats.removed += bodies.len() as u32 - 1;
        i = at;
    }
    *uops = out;
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_read_the_flags_the_architecture_says_and_al_reads_none() {
        let reads: Vec<u8> = (0..16).map(cond_reads).collect();
        assert_eq!(reads, [4, 4, 2, 2, 8, 8, 1, 1, 6, 6, 9, 9, 13, 13, 0, 0]);
    }

    #[test]
    fn the_body_mask_marks_exactly_the_uops_under_a_predicate() {
        let mut p = Uop::new(k::PRED, 0x1000);
        p.imm = 2;
        let plain = Uop::new(k::MOV_IMM, 0x1000);
        let uops = [plain, p, plain, plain, plain, p];
        assert_eq!(
            body_mask(&uops),
            [false, false, true, true, false, false],
            "a body past the end is clamped"
        );
    }

    #[test]
    fn the_switch_off_leaves_the_trace_untouched() {
        let mut p = Uop::new(k::PRED, 0x1000);
        (p.cond, p.imm) = (0, 1);
        let mut uops = vec![
            p,
            Uop::new(k::MOV_IMM, 0x1000),
            p,
            Uop::new(k::MOV_IMM, 0x1004),
        ];
        let before = uops.clone();
        run(&mut uops, &BuildOpts::default());
        assert_eq!(uops, before);
    }
}
