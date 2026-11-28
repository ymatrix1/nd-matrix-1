//! OWNER: W2-PEEP. Dead-flag elimination pass (design note 3.1, WP W2-PEEP).
//!
//! A backward liveness analysis over the four flags of the nibble (I4) turns an S operation
//! whose flags nobody can observe into its non-S twin (generic id minus one: the id is
//! `form * 32 + opcode * 2 + s`, every other field is shared), and a compare or test (`CMP`,
//! `TST`, generic `tst teq cmp cmn`, which have no non-S twin and write nothing else) into the
//! no-op `MOV_REG r0, r0` that covers its instruction (`n` and `pc` kept, so `Uop::n` still sums
//! to the `done` of every exit).
//!
//! * Effects: [`effect::effect`] (reads, writes, kills per uop; every uop that can end the trace
//!   and every `GENERIC*`, `MRS`, `MSR_C` reads all four flags, so a definition is live at
//!   any exit, early exits of stores included).
//! * `PRED`: it reads the flags of its condition; its body runs on one of two paths, so the flags
//!   live in front of the `PRED` are those of the body path and of the skip path joined, and a
//!   definition inside a body is judged on the path where it runs.
//! * The analysis ends with all four flags live (the trace's terminal uop is a barrier anyway).
//! * A definition is dropped only when every flag it may change is dead after it
//!   (`writes & live_out == 0`), which is why the shifter carry of a logic operation that may
//!   be the old C still counts as a write of C.
//!
//! Exits, `done`, `last_pc` and uop positions are untouched (the pass replaces uops one for one).
//! Honours `opts.dead_flags`. Runs after `fuse` and before `pred` (`trace::build`).
//!
//! # Mutation sites (L5, ids `500..=529`)
//!
//! [`MUTANTS`] lists them; `fuse/kills.rs` proves each is killed by this package's suites.
//! The sites sit in `flagopt/effect.rs` (500 to 511, 525) and here.

use super::trace::BuildOpts;
use super::uop::{k, Uop};
use effect::{effect, ALL};

use super::fuse::mutant;

#[cfg(test)]
pub(super) mod cases;
#[cfg(test)]
pub(super) mod diff;
pub mod effect;

/// `(id, name)` of the mutation sites of the dead-flag pass (block `500..=529`).
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (500, "flagopt: a store does not make the flags live"),
    (501, "flagopt: a guard does not make the flags live"),
    (502, "flagopt: GENERIC does not make the flags live"),
    (503, "flagopt: the condition of a PRED is not a read"),
    (504, "flagopt: adc/sbc/rsc do not read C"),
    (505, "flagopt: the skip path of a PRED is ignored"),
    (506, "flagopt: an exit does not make the flags live"),
    (
        507,
        "flagopt: a logic S operation on a register overwrites C",
    ),
    (508, "flagopt: an unrotated immediate overwrites C"),
    (
        509,
        "flagopt: only N and Z are checked before dropping a definition",
    ),
    (510, "flagopt: rrx does not read C"),
    (511, "flagopt: MRS does not make the flags live"),
    (512, "flagopt: a dead compare becomes a move of r1"),
    (513, "flagopt: a dead compare loses its instruction count"),
    (514, "flagopt: the switch is ignored"),
    (515, "flagopt: the twin is the id minus two"),
    (525, "flagopt: GENERIC is taken not to write flags"),
];

/// What one run of the pass did (the "exercised" counters of the tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dropped {
    /// S operations turned into their non-S twin.
    pub s_cleared: u32,
    /// Compares and tests replaced by the no-op.
    pub compares_removed: u32,
}

/// Drop dead flag definitions of `uops` in place.
pub fn run(uops: &mut [Uop], opts: &BuildOpts) {
    run_counted(uops, opts);
}

/// [`run`], reporting what it did.
pub fn run_counted(uops: &mut [Uop], opts: &BuildOpts) -> Dropped {
    let mut dropped = Dropped::default();
    if !opts.dead_flags && !mutant(514) {
        return dropped;
    }
    let n = uops.len();
    // flags live on entry to uop j; at the end of the trace all four are
    let mut live_in = vec![ALL; n + 1];
    for j in (0..n).rev() {
        let u = uops[j];
        if u.kind == k::PRED {
            let end = (j + 1 + u.imm as usize).min(n);
            let reads = effect(&u).reads & if mutant(503) { 0 } else { ALL };
            let skipped = if mutant(505) { 0 } else { live_in[end] };
            live_in[j] = reads | live_in[j + 1] | skipped;
            continue;
        }
        let live_out = live_in[j + 1];
        let mut e = effect(&u);
        let checked = if mutant(509) {
            e.writes & 0b1100
        } else {
            e.writes
        };
        if e.writes != 0 && live_out & checked == 0 && k::family(u.kind) == k::Family::Dp {
            let (d, compare) = replacement(&u);
            dropped.compares_removed += u32::from(compare);
            dropped.s_cleared += u32::from(!compare);
            uops[j] = d;
            e = effect(&d);
        }
        live_in[j] = (live_out & !e.kills) | e.reads;
    }
    dropped
}

/// The uop that does the same without writing flags, and whether the original was a compare or
/// test: the non-S twin of a generic S id, the no-op of a compare or test.
fn replacement(u: &Uop) -> (Uop, bool) {
    let compare = u.kind >= 96 || matches!(u.kind >> 1 & 15, 8..=11);
    if !compare {
        let mut d = *u;
        d.kind -= if mutant(515) { 2 } else { 1 };
        return (d, false);
    }
    let mut d = Uop::new(k::MOV_REG, u.pc);
    d.n = if mutant(513) { 0 } else { u.n };
    if mutant(512) {
        d.rd = 1;
    }
    (d, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_on() -> BuildOpts {
        BuildOpts {
            dead_flags: true,
            ..BuildOpts::default()
        }
    }

    #[test]
    fn the_switch_off_leaves_the_trace_untouched() {
        let mut uops = vec![
            Uop::new(k::dp(1, 4, true), 0x1000),
            Uop::new(k::dp(1, 4, true), 0x1004),
            Uop::exit(0, 0x1004),
        ];
        let before = uops.clone();
        run(&mut uops, &BuildOpts::default());
        assert_eq!(uops, before);
        run(&mut uops, &all_on());
        assert_ne!(uops, before, "with the switch on the first adds is dead");
    }

    #[test]
    fn a_definition_in_front_of_a_barrier_is_kept_and_one_that_is_overwritten_is_dropped() {
        // adds ; adds ; exit: the first is overwritten, the second is live at the exit
        let adds = |pc| Uop::new(k::dp(1, 4, true), pc);
        let mut uops = vec![adds(0x1000), adds(0x1004), Uop::exit(0, 0x1004)];
        let d = run_counted(&mut uops, &all_on());
        assert_eq!(
            d,
            Dropped {
                s_cleared: 1,
                compares_removed: 0
            }
        );
        assert_eq!(
            (uops[0].kind, uops[1].kind),
            (k::dp(1, 4, false), k::dp(1, 4, true))
        );
    }
}
