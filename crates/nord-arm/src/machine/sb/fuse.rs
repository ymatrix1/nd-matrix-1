//! OWNER: W2-PEEP. Fusion pass (design note 3.1, WP W2-PEEP).
//!
//! Rewrites adjacent uop pairs into the fused kinds the executor has arms for
//! (`exec_flow/fused.rs`), each defined as its unfused expansion (`uop::fused::expand`):
//!
//! | Pair | Fused |
//! |---|---|
//! | `CMP_IMM` or `CMP_REG` or `TST_IMM`, then the `GUARD` of the `Bcc` it feeds | `CMP_GUARD_I`, `CMP_GUARD_R`, `TST_GUARD_I` |
//! | `MOV_IMM`, `MOV_IMM` (both below 2^16) | `MOV_PAIR` |
//! | `LDR_IMM`, `LDR_IMM` off one base, no write-back | `LDR_PAIR` |
//!
//! [`uop::fused::fuse_pair`] decides whether a pair has a fused form (adjacency by `pc`, operand
//! packing); this pass decides where to ask. It never pairs a uop that lies in the body of a
//! `PRED` (the skip count of the predicate would be wrong and the fused uop would run
//! unconditionally), never changes the `done` or `last_pc` of an exit (the exits are not
//! touched; a fused guard leaves through the exit of its `GUARD`) and honours `opts.fuse`. The
//! compare of a `CMP_GUARD_*` always writes the flags, and so does its expansion: the flags a
//! fusion "drops" are none, the fused uop is a barrier for `flagopt` (guard).
//!
//! The pairs are the top bigrams of a G0 counter in the design; no counter has run yet
//! (ASSUMED until `arm_sb_bench`), which is why every pair is gated only by `fuse_pair` and a new
//! pair is one more arm there and in `exec_flow/fused.rs`.
//!
//! # Mutation sites (L5, ids `530..=559`)
//!
//! [`MUTANTS`] lists them; `fuse/kills.rs` proves that every site of the three passes
//! (`flagopt` 500..=529, this file 530..=559, `pred` 560..=589) is killed by this package's
//! own suites.

use super::pred::body_mask;
use super::trace::BuildOpts;
use super::uop::fused::fuse_pair;
use super::uop::{k, Uop};

#[cfg(test)]
pub(super) mod cases;
#[cfg(test)]
pub(super) mod diff;
#[cfg(test)]
mod gen;
#[cfg(test)]
pub(super) mod harness;
#[cfg(test)]
mod kills;

/// Mutation site: true only inside `mutants::with(id, ..)` of a test build.
#[cfg(test)]
#[inline(always)]
pub(super) fn mutant(id: u32) -> bool {
    super::mutants::is_active(id)
}

/// Mutation site: false outside a test build, so the wrong branch is dead code there.
#[cfg(not(test))]
#[inline(always)]
pub(super) const fn mutant(_id: u32) -> bool {
    false
}

/// `(id, name)` of the mutation sites of the fusion pass (block `530..=559`).
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (530, "fuse: a pair inside a PRED body is fused"),
    (531, "fuse: the second uop of a fused pair is kept"),
    (532, "fuse: the exit of a fused guard is lost"),
    (533, "fuse: the switch is ignored"),
    (
        534,
        "fuse: the fused uop takes the pc of the second instruction",
    ),
    (535, "fuse: the fused uop counts one instruction"),
    (536, "fuse: the offsets of a load pair are swapped"),
    (537, "fuse: the destinations of a move pair are swapped"),
    (538, "fuse: the condition of a fused guard is inverted"),
    (
        539,
        "fuse: a fused guard is made of the compare two uops back",
    ),
];

/// What one run of the pass did, by fused kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fused {
    pub cmp_guard_i: u32,
    pub cmp_guard_r: u32,
    pub tst_guard_i: u32,
    pub mov_pair: u32,
    pub ldr_pair: u32,
}

impl Fused {
    /// Fused uops made.
    pub fn total(&self) -> u32 {
        self.cmp_guard_i + self.cmp_guard_r + self.tst_guard_i + self.mov_pair + self.ldr_pair
    }

    fn count(&mut self, kind: u8) {
        match kind {
            k::CMP_GUARD_I => self.cmp_guard_i += 1,
            k::CMP_GUARD_R => self.cmp_guard_r += 1,
            k::TST_GUARD_I => self.tst_guard_i += 1,
            k::MOV_PAIR => self.mov_pair += 1,
            _ => self.ldr_pair += 1,
        }
    }
}

/// Fuse `uops` in place.
pub fn run(uops: &mut Vec<Uop>, opts: &BuildOpts) {
    run_counted(uops, opts);
}

/// The fused uop for `uops[i]` and its successor, if any (mutation sites included).
fn fused_at(uops: &[Uop], body: &[bool], i: usize) -> Option<Uop> {
    let (a, b) = (uops.get(i)?, uops.get(i + 1)?);
    if (body[i] || body[i + 1]) && !mutant(530) {
        return None;
    }
    let mut f = fuse_pair(a, b)?;
    if mutant(532) && f.kind <= k::TST_GUARD_I {
        f.rs = 0;
    }
    if mutant(534) {
        f.pc = b.pc;
    }
    if mutant(535) {
        f.n = 1;
    }
    if mutant(536) && f.kind == k::LDR_PAIR {
        f.imm = f.imm.rotate_left(16);
    }
    if mutant(537) && f.kind == k::MOV_PAIR {
        (f.rd, f.rn) = (f.rn, f.rd);
    }
    if mutant(538) && f.kind <= k::TST_GUARD_I {
        f.cond ^= 1;
    }
    if mutant(539) && f.kind <= k::TST_GUARD_I && i > 0 {
        f.rn = uops[i - 1].rn;
    }
    Some(f)
}

/// [`run`], reporting what it did.
pub fn run_counted(uops: &mut Vec<Uop>, opts: &BuildOpts) -> Fused {
    let mut done = Fused::default();
    if !opts.fuse && !mutant(533) {
        return done;
    }
    let body = body_mask(uops);
    let mut out = Vec::with_capacity(uops.len());
    let mut i = 0;
    while i < uops.len() {
        match fused_at(uops, &body, i) {
            Some(f) => {
                done.count(f.kind);
                out.push(f);
                if mutant(531) {
                    out.push(uops[i + 1]);
                }
                i += 2;
            }
            None => {
                out.push(uops[i]);
                i += 1;
            }
        }
    }
    *uops = out;
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_off_leaves_the_trace_untouched() {
        let mut uops = vec![Uop::new(k::CMP_IMM, 0x1000), Uop::new(k::GUARD, 0x1004)];
        let before = uops.clone();
        run(&mut uops, &BuildOpts::default());
        assert_eq!(uops, before);
        let on = BuildOpts {
            fuse: true,
            ..BuildOpts::default()
        };
        run(&mut uops, &on);
        assert_eq!((uops.len(), uops[0].kind), (1, k::CMP_GUARD_I));
    }

    #[test]
    fn a_fused_uop_is_not_fused_again_and_pairs_do_not_overlap() {
        // mov mov mov: the first two fuse, the third stays; a MOV_PAIR is never a candidate
        let mov = |pc, rd| {
            let mut u = Uop::new(k::MOV_IMM, pc);
            u.rd = rd;
            u
        };
        let mut uops = vec![
            mov(0x1000, 1),
            mov(0x1004, 2),
            mov(0x1008, 3),
            mov(0x100C, 4),
        ];
        let on = BuildOpts {
            fuse: true,
            ..BuildOpts::default()
        };
        let d = run_counted(&mut uops, &on);
        assert_eq!((d.mov_pair, d.total(), uops.len()), (2, 2, 2));
        assert_eq!(run_counted(&mut uops, &on).total(), 0);
    }
}
