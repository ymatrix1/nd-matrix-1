//! OWNER: W2-PEEP (tests). The dead-flag pass against the unpassed trace and the interpreter on
//! random states and whole programs, including the flags it drops (the final flag nibble is part
//! of every comparison, and a flag live at an exit shows there), and the conservative cases: a
//! flag live at an early exit of a store, a guard, a `GENERIC`, or through a predicate.

use crate::machine::sb::fuse::diff::{l0, l1, need_l0};
use crate::machine::sb::fuse::harness::{check_pair, Passes, Tally};
use crate::machine::sb::testgen::asm::{self as a, AL};
use crate::machine::sb::testgen::CODE;
use crate::machine::sb::testkit::{fuzz_cases, fuzz_seed};
use crate::machine::sb::trace::testexec::suite;

/// The sweep of the kill run of `kills.rs`.
pub(in crate::machine::sb) fn probe() {
    let t = l0(8, Passes::DEAD, 0xDEAD);
    assert!(t.s_removed > 5, "dead flags removed: {}", t.s_removed);
}

/// Run `words` from every start in `0..starts` under 64 random states with the pass on.
fn sweep_words(words: &[u32], starts: usize, tally: &mut Tally) {
    for start in 0..starts {
        for seed in 0..64u64 {
            let extra = [0u64, 2, 30][(seed % 3) as usize];
            check_pair(
                words,
                start,
                seed * 7 + start as u64,
                extra,
                Passes::DEAD,
                tally,
            )
            .unwrap_or_else(|e| panic!("start {start} seed {seed}: {e}"));
        }
    }
}

fn at(i: usize) -> u32 {
    CODE + 4 * i as u32
}

suite! {
    #[test]
    fn dropping_dead_flags_equals_the_unpassed_trace_and_the_interpreter_on_random_states() {
        let t = l0(fuzz_cases(60), Passes::DEAD, fuzz_seed() + 0xDF);
        t.run.assert_exercised("dead flags L0", &need_l0(t.pairs));
        assert!(t.pairs >= 1000, "{} trace pairs", t.pairs);
        assert!(t.s_removed >= 300, "S operations and compares removed: {}", t.s_removed);
        assert_eq!((t.fused_total(), t.preds_removed, t.uops_saved), (0, 0, 0));
    }

    #[test]
    fn programs_with_dead_flags_dropped_end_in_the_state_of_the_interpreter() {
        let t = l1(fuzz_cases(60) / 3 + 1, Passes::DEAD, fuzz_seed() + 0xD1);
        assert!(t.entered > 100, "{} traces entered", t.entered);
    }

    #[test]
    fn a_definition_before_a_guard_a_store_or_a_generic_is_live_and_the_exit_sees_it() {
        // adds r0,r0,r1 ; <barrier> ; adds r2,r2,r3 ; ... : the first adds is overwritten by the
        // second, so only the barrier (an early exit through which the flags must be exact) keeps it
        let adds1 = a::dp_shift_imm(4, true, 0, 0, 1, 0, 0);
        let adds2 = a::dp_shift_imm(4, true, 2, 2, 3, 0, 0);
        let barriers: [(&str, u32); 4] = [
            ("guard", a::branch(1, false, at(1), at(9))),
            ("store", a::str(4, 8, 0)),
            ("generic", 0xE000_0090 | 5 << 16 | 6 << 8 | 7), // mul r5,r7,r6
            ("mrs", a::mrs(5, false)),
        ];
        for (name, barrier) in barriers {
            let mut words = vec![adds1, barrier, adds2];
            words.extend([a::NOP; 8]);
            words.push(a::branch(AL, false, at(words.len()), at(words.len())));
            let mut t = Tally::default();
            sweep_words(&words, 1, &mut t);
            assert_eq!(t.s_removed, 0, "{name}: the first adds is live at the exit");
            assert!(t.run.flag_changes > 20, "{name}: flags changed");
        }
    }

    #[test]
    fn a_definition_overwritten_before_every_exit_is_dropped_and_nothing_shows() {
        // adds ; subs ; subs : only the last one is live
        let words = [
            a::dp_shift_imm(4, true, 0, 0, 1, 0, 0),
            a::dp_shift_imm(2, true, 2, 2, 3, 0, 0),
            a::sub_imm(true, 4, 4, 5),
            a::NOP,
            a::branch(AL, false, at(4), at(4)),
        ];
        let mut t = Tally::default();
        sweep_words(&words, 1, &mut t);
        assert_eq!(t.s_removed, 64 * 2, "the adds and the subs are dropped in every run");
        assert!(t.run.flag_changes > 40);
    }

    #[test]
    fn a_flag_read_through_a_predicate_or_a_carry_in_keeps_its_definition() {
        // subs r0,r0,r1 ; addeq r2,r2,#1 ; cmp r4,#1 ; : the eq reads the Z of the subs
        let (subs, addeq, cmp) = (
            a::dp_shift_imm(2, true, 0, 0, 1, 0, 0),
            a::dp_imm(0, 4, false, 2, 2, 0, 1),
            a::cmp_imm(4, 1),
        );
        // adds r0,r0,r1 ; adc r2,r2,#0 ; cmp r4,#1
        let (adds, adc) = (a::dp_shift_imm(4, true, 0, 0, 1, 0, 0), a::dp_imm(AL, 5, false, 2, 2, 0, 0));
        for (name, words) in [("predicate", [subs, addeq, cmp]), ("carry in", [adds, adc, cmp])] {
            let mut w = words.to_vec();
            w.extend([a::NOP; 4]);
            w.push(a::branch(AL, false, at(w.len()), at(w.len())));
            let mut t = Tally::default();
            sweep_words(&w, 1, &mut t);
            assert_eq!(t.s_removed, 0, "{name}: the definition is read");
        }
    }
}
