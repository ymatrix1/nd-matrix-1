//! OWNER: W2-PEEP (tests). The sweeps of the differential harness (`harness.rs`) shared by the
//! three passes, and the fusion's own differential tests: fused traces against the unfused ones
//! and against the interpreter on random states (L0 style) and whole programs on the interpreter
//! and on fused traces (L1 style), each ending with the counters that prove fusion fired.

use super::gen::program;
use super::harness::{check_pair, program_check, Passes, Tally};
use crate::machine::sb::testkit::{fuzz_cases, fuzz_seed, Need, Rng};
use crate::machine::sb::trace::testexec::suite;
use crate::machine::sb::uop::k;

/// Budgets above `n_guest` the L0 cases run with: none, a few instructions, whole laps.
const EXTRA: [u64; 6] = [0, 0, 1, 3, 40, 200];

/// L0: `programs` generated programs, a trace at about 25 of their pcs each, run from a random
/// state against the unpassed trace and the interpreter.
pub(in crate::machine::sb) fn l0(programs: u64, passes: Passes, seed: u64) -> Tally {
    let mut t = Tally::default();
    for p in 0..programs {
        let mut rng = Rng::new(seed + p);
        let len = 50 + rng.below(70) as usize;
        let words = program(&mut rng, len);
        let step = 1 + rng.below(3) as usize;
        for (n, start) in (0..words.len()).step_by(step).take(30).enumerate() {
            let extra = EXTRA[(p as usize + n) % EXTRA.len()];
            let case = (seed + p) * 1000 + n as u64;
            if let Err(e) = check_pair(&words, start, case, extra, passes, &mut t) {
                panic!("program {} start {start} extra {extra}: {e}", seed + p);
            }
        }
    }
    t
}

/// L1: the same programs as whole runs on budgets that expire at every uop position.
pub(in crate::machine::sb) fn l1(programs: u64, passes: Passes, seed: u64) -> Tally {
    const BUDGETS: [u64; 9] = [1, 2, 3, 5, 9, 17, 40, 120, 333];
    let mut t = Tally::default();
    for p in 0..programs {
        let mut rng = Rng::new(seed + p);
        let len = 50 + rng.below(70) as usize;
        let words = program(&mut rng, len);
        for (n, budget) in BUDGETS.into_iter().enumerate() {
            if let Err(e) = program_check(&words, seed + p + n as u64, budget, passes, &mut t) {
                panic!("program {} budget {budget}: {e}", seed + p);
            }
        }
    }
    t
}

/// What an L0 sweep must have done for "no difference" to mean something.
pub(in crate::machine::sb) fn need_l0(cases: u64) -> Need {
    Need {
        cases,
        retired: 4 * cases,
        loads: cases / 4,
        stores: cases / 8,
        jumps: cases / 10,
        reg_writes: cases / 2,
        flag_changes: cases / 4,
        max_idle_pct: 30,
        ..Need::default()
    }
}

/// The L0 sweep of the kill run of `kills.rs`: small, fixed seed, fusion only.
pub(in crate::machine::sb) fn probe() {
    let t = l0(8, Passes::FUSE, 0x5EED);
    assert!(t.fused_total() > 10, "fusion fired {}", t.fused_total());
    let t = l1(2, Passes::FUSE, 0x5EEE);
    assert!(t.entered > 0);
}

suite! {
    #[test]
    fn fused_traces_equal_the_unfused_ones_and_the_interpreter_on_random_states() {
        let programs = fuzz_cases(60);
        let t = l0(programs, Passes::FUSE, fuzz_seed() + 0xF05E);
        t.run.assert_exercised("fusion L0", &need_l0(t.pairs));
        assert!(t.pairs >= 1000, "{} trace pairs", t.pairs);
        // every fused kind was built, entered, and the compare-and-branch pairs went both ways
        for (kind, name) in [
            (k::CMP_GUARD_I, "cmp #imm + bcc"),
            (k::CMP_GUARD_R, "cmp rm + bcc"),
            (k::TST_GUARD_I, "tst #imm + bcc"),
            (k::MOV_PAIR, "mov pair"),
            (k::LDR_PAIR, "ldr pair"),
        ] {
            let n = t.fused[(kind - k::CMP_GUARD_I) as usize];
            assert!(n >= 30, "{name}: {n} fused uops in traces that ran");
        }
        assert!(t.fused_taken >= 100 && t.fused_fell >= 100, "{} {}", t.fused_taken, t.fused_fell);
        assert!(t.laps > 5, "loops re-entered: {}", t.laps);
        // one fusion saves exactly one uop: the second uop of a pair is gone
        assert_eq!(t.uops_saved, t.fused_total(), "uops saved per fused uop");
        assert_eq!((t.s_removed, t.preds_removed), (0, 0), "only fusion was on");
    }

    #[test]
    fn programs_on_fused_traces_end_in_the_state_of_the_interpreter_at_every_budget() {
        let t = l1(fuzz_cases(60) / 3 + 1, Passes::FUSE, fuzz_seed() + 0xF1);
        assert!(t.entered > 100, "{} traces entered", t.entered);
        assert!(t.fused_total() > 50, "{} fused uops entered", t.fused_total());
    }
}
