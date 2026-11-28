//! OWNER: W3-VERIF (scaffold written by S0). Layers L0 and L1 of the verification plan: every
//! tier against the interpreter.
//!
//! Spec (design note 8, WP W3-VERIF):
//!
//! * L0, per uop: random state x random word per family against `Cpu::step` on a logging bus
//!   (registers, flags, the ordered access log, `bus.pc`), traces of 1 to 3 instructions.
//!   [`l0_case`] is one case and [`l0_sweep`] the CI-bound loop (`NS4_FUZZ_CASES`, default 60,
//!   each unit 20 000 words per family; the local gate is 1 000 000 words per family).
//! * L1, programs, three-way: plain `Cpu::step` (T0), the predecode loop (T1), the trace tier
//!   built per pc ([`drive_traces`]) and the full tier driver (`run_sb_full` with
//!   `SbConfig::on()`), on the generator of `testgen`, compared on the whole [`Snapshot`] (CPU,
//!   `bus.pc`, memory hash, device counters, messages, auto-mapped pages, tick clock) and the
//!   stop reason. W3-VERIF adds the budget sweeps (1..7 and 1..400, so the budget expires at
//!   every uop position), `HOT` in {0, 1, 24}, trace caps {1, 2, 3, 128}, and `until` and `stop`
//!   inside traces.
//!
//! The first test, `skeleton_stack_equals_the_interpreter_on_a_synthetic_program`, is the proof
//! that the always-correct baseline holds: with every lowerer a stub, the whole stack is the
//! interpreter.

#![allow(dead_code)]

use super::config::SbConfig;
use super::run::run_sb_full;
use super::testgen::{drive_traces, lowered_vs_step, machine_with, random_program, CODE};
use super::testkit::{fuzz_cases, fuzz_seed, random_cpu, snapshot, Rng, Snapshot};
use super::SbState;
use crate::cpu::Cpu;
use crate::machine::Stop;

/// A tier configuration of the three-way comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier {
    /// `Cpu::step` only (`predecode = false`): the oracle.
    T0,
    /// The predecode loop (`run_cached`).
    T1,
    /// Traces built per pc and run by `exec::run_sb`, no cache (`testgen::drive_traces`).
    Traces,
    /// The tier driver, `run_sb_full` with `SbConfig::on()`.
    Full,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Tier::T0, Tier::T1, Tier::Traces, Tier::Full];
}

/// What a run of `budget` instructions left behind.
pub(crate) type Outcome = (Snapshot, u64, Stop);

/// Run `words` (seeded as in `testgen::machine_with`) on `tier` for `budget` instructions.
pub(crate) fn run_tier(
    tier: Tier,
    words: &[u32],
    seed: u64,
    budget: u64,
    until: u32,
    stop_pc: Option<u32>,
) -> Outcome {
    let mut m = machine_with(words, seed, tier != Tier::T0);
    let (n, why) = match tier {
        Tier::T0 | Tier::T1 => m.run(budget, until, stop_pc),
        Tier::Traces => drive_traces(&mut m, budget, until, stop_pc),
        Tier::Full => {
            let mut sb = SbState {
                cfg: SbConfig::on(),
                ..SbState::default()
            };
            run_sb_full(&mut m, &mut sb, budget, until, stop_pc)
        }
    };
    let fault = if let Stop::Fault(f) = why {
        Some(f)
    } else {
        None
    };
    (snapshot(&m, fault), n, why)
}

/// One L1 case: every tier against T0 on the same program, seed, budget and stop addresses.
/// `Err` names the tier and the first differing field.
pub(crate) fn l1_case(
    words: &[u32],
    seed: u64,
    budget: u64,
    until: u32,
    stop_pc: Option<u32>,
) -> Result<(), String> {
    let (want, n0, why0) = run_tier(Tier::T0, words, seed, budget, until, stop_pc);
    for tier in &Tier::ALL[1..] {
        let (got, n, why) = run_tier(*tier, words, seed, budget, until, stop_pc);
        if (n, why) != (n0, why0) {
            return Err(format!(
                "{tier:?}: ran {n} insns and stopped with {why:?}, T0 ran {n0} and {why0:?}"
            ));
        }
        if let Some(d) = want.diff(&got) {
            return Err(format!("{tier:?} after {n} insns: {d}"));
        }
    }
    Ok(())
}

/// One L0 case: `word` lowered and executed against `Cpu::step` on `cpu`.
pub(crate) fn l0_case(word: u32, cpu: &Cpu, seed: u64) -> Result<(), String> {
    lowered_vs_step(word, cpu, seed)
}

/// `cases` L0 cases of random words and states from `seed` (the scaffold of the per-family sweep).
pub(crate) fn l0_sweep(cases: u64, seed: u64) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    for i in 0..cases {
        let pc = 0x1000 + 4 * rng.below(0x400);
        let word = super::testgen::random_word(&mut rng, 0, 64);
        let cpu = random_cpu(&mut rng, pc);
        l0_case(word, &cpu, seed ^ i).map_err(|e| format!("case {i}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testgen::tiny_loop;

    /// The proof that the skeleton changes nothing: the synthetic program (calls, a conditional
    /// loop, stores and loads) on every tier equals the interpreter, for every budget that
    /// expires at each position of the loop.
    #[test]
    fn skeleton_stack_equals_the_interpreter_on_a_synthetic_program() {
        let words = tiny_loop();
        for budget in 1..=60 {
            l1_case(&words, 7, budget, 0, None).unwrap_or_else(|e| panic!("budget {budget}: {e}"));
        }
        // the loop leaves r0 = 258 and spins at word 6 on every tier
        let (snap, n, why) = run_tier(Tier::Traces, &words, 7, 100, 0, None);
        assert_eq!((n, why), (100, Stop::Budget));
        assert_eq!(
            (snap.cpu.r[0], snap.cpu.r[1], snap.cpu.r[15]),
            (258, 0, CODE + 24)
        );
    }

    #[test]
    fn the_stop_addresses_are_honoured_by_every_tier() {
        let words = tiny_loop();
        for (until, stop) in [
            (CODE + 12, None),
            (0, Some(CODE + 28)),
            (CODE + 4, Some(CODE + 8)),
        ] {
            l1_case(&words, 3, 100, until, stop).unwrap();
        }
        let (_, n, why) = run_tier(Tier::Full, &words, 3, 100, CODE + 28, None);
        assert_eq!(why, Stop::Until);
        assert!(n > 0 && n < 100);
    }

    #[test]
    fn random_programs_agree_on_every_tier() {
        let base = fuzz_seed();
        for case in 0..fuzz_cases(60) {
            let mut rng = Rng::new(base ^ case.wrapping_mul(0x9E37_79B9));
            let words = random_program(&mut rng, 64);
            let budget = 1 + u64::from(rng.below(400));
            l1_case(&words, case, budget, 0, None)
                .unwrap_or_else(|e| panic!("program {case} budget {budget}: {e}"));
        }
    }

    #[test]
    fn the_comparator_sees_a_divergence_in_any_part_of_the_outcome() {
        let words = tiny_loop();
        let (want, n, why) = run_tier(Tier::T0, &words, 1, 50, 0, None);
        let (other_seed, _, _) = run_tier(Tier::T0, &words, 2, 50, 0, None);
        assert!(
            want.diff(&other_seed).unwrap().contains("r"),
            "different registers"
        );
        let (shorter, n2, _) = run_tier(Tier::T0, &words, 1, 40, 0, None);
        assert!(
            n2 < n && want.diff(&shorter).is_some(),
            "different progress"
        );
        let (same, n3, why3) = run_tier(Tier::T1, &words, 1, 50, 0, None);
        assert_eq!((n3, why3, want.diff(&same)), (n, why, None));
    }

    /// The fuzz is not vacuous: a quarter of the generated programs retire at least 20
    /// instructions on the oracle before a fault or the budget ends them.
    #[test]
    fn the_generated_programs_run_long_enough_to_mean_something() {
        let mut long = 0;
        for case in 0..100u64 {
            let words = random_program(&mut Rng::new(case.wrapping_mul(0x9E37_79B9)), 64);
            let (_, n, _) = run_tier(Tier::T0, &words, case, 400, 0, None);
            long += u32::from(n >= 20);
        }
        assert!(
            long >= 25,
            "only {long} of 100 programs ran 20 instructions"
        );
    }

    #[test]
    fn l0_sweep_passes_on_the_skeleton_and_l0_case_reports_a_wrong_word() {
        l0_sweep(fuzz_cases(60) * 10, fuzz_seed() + 1).unwrap();
        let cpu = random_cpu(&mut Rng::new(2), 0x1000);
        l0_case(0xE3A0_0005, &cpu, 1).unwrap();
    }
}
