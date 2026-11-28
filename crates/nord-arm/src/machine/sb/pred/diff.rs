//! OWNER: W2-PEEP (tests). The predicate grouping against the unpassed trace and the interpreter
//! on random states and whole programs: every condition, runs of 2 to 10 members, bodies that
//! store, load, write the flags or leave through an early exit, and the merged predicate's skip
//! count on both paths (condition true and false) of every run.

use crate::machine::sb::fuse::diff::{l0, l1, need_l0};
use crate::machine::sb::fuse::harness::{check_pair, Passes, Tally};
use crate::machine::sb::testgen::asm::{self as a, AL};
use crate::machine::sb::testgen::CODE;
use crate::machine::sb::testkit::{fuzz_cases, fuzz_seed};
use crate::machine::sb::trace::testexec::suite;

/// The sweep of the kill run of `kills.rs`.
pub(in crate::machine::sb) fn probe() {
    let t = l0(8, Passes::PRED, 0xBEEF);
    assert!(
        t.preds_removed > 5,
        "predicates removed: {}",
        t.preds_removed
    );
    let t = l1(2, Passes::PRED, 0xBEEE);
    assert!(t.entered > 0);
}

/// `cond` applied to the word `w`.
fn under(cond: u32, w: u32) -> u32 {
    w & 0x0FFF_FFFF | cond << 28
}

suite! {
    #[test]
    fn grouped_predicates_equal_the_unpassed_trace_and_the_interpreter_on_random_states() {
        let t = l0(fuzz_cases(60), Passes::PRED, fuzz_seed() + 0xBE);
        t.run.assert_exercised("predicate grouping L0", &need_l0(t.pairs));
        assert!(t.pairs >= 1000, "{} trace pairs", t.pairs);
        assert!(t.preds_removed >= 300, "predicates removed: {}", t.preds_removed);
        assert_eq!(t.uops_saved, t.preds_removed, "a merge removes exactly the PRED uop");
        assert_eq!((t.fused_total(), t.s_removed), (0, 0));
    }

    #[test]
    fn programs_with_grouped_predicates_end_in_the_state_of_the_interpreter() {
        let t = l1(fuzz_cases(60) / 3 + 1, Passes::PRED, fuzz_seed() + 0xB1);
        assert!(t.entered > 100, "{} traces entered", t.entered);
    }

    #[test]
    fn a_run_of_one_condition_runs_or_skips_whole_for_every_condition_and_flag_state() {
        // addX r1,r1,#1 ; subX r2,r2,#3 ; movX r3,#9 ; strX r4,[r8] ; ldrX r5,[r8,#4] ; orrX ...
        for cond in 0..14u32 {
            let words: Vec<u32> = [
                a::add_imm(false, 1, 1, 1),
                a::sub_imm(false, 2, 2, 3),
                a::mov_imm(3, 9),
                a::str(4, 8, 0),
                a::ldr(5, 8, 4),
                a::dp_imm(AL, 12, false, 6, 6, 0, 0x30),
            ]
            .map(|w| under(cond, w))
            .into_iter()
            .chain([a::NOP, a::branch(AL, false, CODE + 24 + 4, CODE + 24 + 4)])
            .collect();
            let mut t = Tally::default();
            for seed in 0..48u64 {
                check_pair(&words, 0, seed * 13 + u64::from(cond), 0, Passes::PRED, &mut t)
                    .unwrap_or_else(|e| panic!("cond {cond} seed {seed}: {e}"));
            }
            assert_eq!(t.preds_removed, 48 * 5, "cond {cond}: six members merged into one");
            let run = &t.run;
            assert!(run.stores > 5 && run.loads > 5 && run.reg_writes > 5,
                "cond {cond}: the run ran in some states: {run:?}");
        }
    }

    #[test]
    fn a_member_that_writes_the_flags_is_last_and_the_next_one_sees_the_new_flags() {
        // subeq ; addseq (a writer closes the run) ; addne ; addeq ; subeq ; subseq (closes) ; addeq
        let words = [
            under(0, a::sub_imm(false, 2, 2, 1)),
            under(0, a::add_imm(true, 1, 1, 0xFF)),
            under(1, a::add_imm(false, 3, 3, 1)),
            under(0, a::add_imm(false, 7, 7, 1)),
            under(0, a::sub_imm(false, 5, 5, 1)),
            under(0, a::sub_imm(true, 6, 6, 1)),
            under(0, a::add_imm(false, 4, 4, 1)),
            a::NOP,
            a::branch(AL, false, CODE + 32, CODE + 32),
        ];
        let mut t = Tally::default();
        for seed in 0..96u64 {
            check_pair(&words, 0, seed, 0, Passes::PRED, &mut t).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        }
        assert_eq!(t.preds_removed, 96 * 3, "runs of 2 and 3 were grouped, the rest stayed");
    }
}
