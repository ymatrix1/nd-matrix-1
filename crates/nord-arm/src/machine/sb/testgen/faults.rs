//! OWNER: W1-VGEN. Words that fault, for the structured programs (design note 8, L1: "undefined
//! words"; W2-XCORE: "fault inside a trace"). Three places hold them: a construct of the body
//! ([`Gen::fault_site`](super::blocks::Gen)), the callees ([`plant`]) and the pool of patch words
//! a store can copy over the program (`program.rs`), so a fault is reached straight on, inside a
//! callee, after a loop, or only once a store has rewritten the code (decoded code gone stale
//! first, then faulting).
//!
//! A fault word is guarded by a condition two times in three: the same word faults in some runs
//! and is skipped in others, so a trace holds a fault that is not always taken.

use super::asm::{SWI, UNDEFINED, WFI};
use super::parts::{with_cond, CONDS};
use crate::machine::sb::testkit::Rng;

/// A word that ends a run with a fault: an undefined word of the permanently undefined space
/// (3 in 7), `swi #n` (2 in 7) or `wfi` (2 in 7). The interpreter reports them as
/// `Fault::Undefined`, `Fault::Swi` and `Fault::Wfi`.
///
/// VERIFIED: `UNDEFINED | imm12 << 8 | imm4` (`cond 0111 1111 imm12 1111 imm4`, the `UDF` space)
/// is `Fault::Undefined` with the word as `insn`, `SWI | n` is `Fault::Swi { imm: n }` and
/// `WFI` is `Fault::Wfi` on `Cpu::step`, checked by `the_fault_words_fault_the_way_their_names_say`
/// over 600 draws (no exception).
/// ASSUMED: the 3:2:2 mix of the kinds, only so that each kind turns up in a few programs of 200
/// (`generated_programs_fault_now_and_then_with_every_kind_of_fault_word`).
pub(super) fn fault_word(rng: &mut Rng) -> u32 {
    match rng.below(7) {
        0..=2 => UNDEFINED | rng.below(0x1000) << 8 | rng.below(16),
        3..=4 => SWI | rng.below(0x100),
        _ => WFI,
    }
}

/// [`fault_word`], conditional two times in three.
pub(super) fn guarded_fault(rng: &mut Rng) -> u32 {
    let w = fault_word(rng);
    if rng.chance(3) {
        w
    } else {
        with_cond(w, rng.pick(&CONDS))
    }
}

/// Whether `w`, whatever its condition, is a word of [`fault_word`]'s three kinds.
pub(super) fn is_fault_word(w: u32) -> bool {
    let (undefined, swi, wfi) = (
        w >> 20 & 0xFF == 0x7F && w >> 4 & 0xF == 0xF,
        w >> 24 & 0xF == 0xF,
        w & 0x0FFF_FFFF == WFI & 0x0FFF_FFFF,
    );
    undefined || swi || wfi
}

/// A word a fault word may take the place of in a callee: unconditional, not a branch, not a
/// block transfer, and no register of `r8` and above in the `rd` field, so the callee keeps its
/// return, its frame and its loop counter (`r12`) and ends the way it did.
pub(super) fn is_replaceable(w: u32) -> bool {
    w >> 28 == 14 && (w >> 25 & 7) < 4 && (w >> 12 & 0xF) < 8
}

/// Put a guarded fault word in place of one replaceable word of `f` (a callee), if it has one.
pub(super) fn plant(rng: &mut Rng, f: &mut [u32]) {
    let at: Vec<usize> = (0..f.len()).filter(|i| is_replaceable(f[*i])).collect();
    if !at.is_empty() {
        f[rng.pick(&at)] = guarded_fault(rng);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Fault;
    use crate::machine::sb::testgen::parts::{callees, function};
    use crate::machine::sb::testgen::program::FUNC_AT;
    use crate::machine::sb::testkit::{oracle_trace, random_cpu};

    #[test]
    fn the_fault_words_fault_the_way_their_names_say() {
        let mut rng = Rng::new(3);
        let (mut undefined, mut swi, mut wfi) = (0, 0, 0);
        for case in 0..600 {
            let w = fault_word(&mut rng);
            let cpu = random_cpu(&mut rng, 0x1000);
            let t = oracle_trace(&cpu, &[w], case, 1);
            match t.fault {
                Some(Fault::Undefined { insn, .. }) => {
                    assert_eq!(insn, w);
                    undefined += 1;
                }
                Some(Fault::Swi { imm, .. }) => {
                    assert_eq!(imm, w & 0xFF_FFFF);
                    swi += 1;
                }
                Some(Fault::Wfi) => wfi += 1,
                other => panic!("{w:#010x} gave {other:?}"),
            }
        }
        assert!(
            undefined > 200 && swi > 100 && wfi > 100,
            "{undefined}/{swi}/{wfi}"
        );
    }

    #[test]
    fn a_guarded_fault_is_skipped_when_its_condition_fails() {
        let (mut rng, mut taken, mut skipped, mut always) = (Rng::new(5), 0, 0, 0);
        for case in 0..600 {
            let w = guarded_fault(&mut rng);
            let mut cpu = random_cpu(&mut rng, 0x1000);
            cpu.set_cpsr(cpu.cpsr() & 0x0FFF_FFFF | (rng.below(16) << 28));
            let t = oracle_trace(&cpu, &[w], case, 1);
            always += u32::from(w >> 28 == 14);
            match t.fault {
                Some(_) => taken += 1,
                None => {
                    assert_eq!(t.cpu.r[15], 0x1004, "a failed condition is a nop");
                    skipped += 1;
                }
            }
        }
        assert!(
            taken > 150 && skipped > 100,
            "{taken} taken, {skipped} skipped"
        );
        assert!(
            (120..280).contains(&always),
            "one word in three is unconditional: {always}"
        );
    }

    #[test]
    fn planting_replaces_at_most_one_plain_word_and_never_a_return_or_a_loop_word() {
        let mut planted = 0;
        for seed in 0..300u64 {
            let mut rng = Rng::new(seed);
            for (kind, at) in FUNC_AT.iter().enumerate() {
                let before = function(&mut rng, kind, *at, FUNC_AT[0]);
                let mut after = before.clone();
                plant(&mut rng, &mut after);
                let changed: Vec<usize> = (0..before.len())
                    .filter(|i| before[*i] != after[*i])
                    .collect();
                assert!(changed.len() <= 1, "kind {kind}: {changed:?}");
                for i in &changed {
                    assert!(
                        is_replaceable(before[*i]),
                        "kind {kind}: {:#010x} replaced",
                        before[*i]
                    );
                    assert!(*i + 1 < before.len(), "the return word stays");
                    planted += 1;
                }
            }
        }
        assert!(planted > 1000, "{planted} words replaced in 1800 callees");
    }

    #[test]
    fn callees_of_the_structured_generator_carry_faults_now_and_then() {
        let (mut faulty, mut total) = (0, 0);
        for seed in 0..200 {
            for (_, f) in callees(&mut Rng::new(seed)) {
                total += 1;
                faulty += usize::from(f.iter().any(|w| is_fault_word(*w)));
            }
        }
        assert!(
            faulty * 100 > total * 2 && faulty * 100 < total * 8,
            "{faulty} of {total} callees"
        );
    }
}
