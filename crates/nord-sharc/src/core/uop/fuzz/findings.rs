//! What the fuzz found, each pinned to a reproduction that always runs.
//!
//! OWNER: W3-FUZZ. A finding here is a difference between the engine and the interpreter, or a
//! performance cliff, that the fuzz reproduced and that this package may not fix (the code is
//! another package's). The test prints what it saw and passes while the finding stands;
//! `NS4_FUZZ_STRICT=1` turns the same test into the check that the finding is fixed. When the
//! owner fixes the code, the strict assertion holds and the gate can go.
use super::checked::{pass_core, random_states};
use super::knobs::strict;
use super::scenario::{run, Irq, Scenario};
use super::shapes::straight;
use super::sieve::Fam;
use crate::core::support::CODE;
use crate::core::uop::config::UopConfig;
use crate::core::uop::verify::gen::Rng;
use crate::runner::uop_pass::arm_uop;

/// FINDING 1 (`irq_due`, found by `checked::random_states`, seed `0xac5df5d8fe2fe656`, state 1):
/// a fixed-point shift with an arithmetic exception, run for two steps from a random state, leaves
/// `irq.due == 0` in the interpreter and `u64::MAX` in the engine. `irq.due` is the cached next
/// event deadline, rebuilt at the next step, so no architectural state differs and the next
/// interrupt is still seen on time; but `tv::validate` and `shadow_run` compare it and report a
/// `Mismatch` on a correct engine, so a shadowed pass can heal a core that was right.
///
/// Reproduction: [`PROGRAM`] on a `pass_core` of that seed, `random_states(c, 2, seed, 12)`.
const PROGRAM: [u16; 19] = [
    0x023e, 0x0802, 0x000c, 0xcfbf, 0x0d04, 0x0000, 0x000c, 0x0c00, 0x0100, 0x0008, 0x0d05, 0x0000,
    0x0004, 0xc0bf, 0xc97f, 0xc3bf, 0xca7f, 0xcf3f, 0xcfa8,
];
/// Seed of finding 1.
const SEED: u64 = 0xac5d_f5d8_fe2f_e656;

#[test]
fn finding_1_irq_due_differs_after_an_exception_raising_shift_on_a_random_state() {
    let (mut c, cfg) = pass_core(&PROGRAM, SEED, false);
    c.pc = CODE;
    arm_uop(&mut c, UopConfig::default(), cfg.idle_pc).unwrap();
    // strict: any difference panics; otherwise the `irq_due`-only one is counted
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        random_states(&mut c, 2, SEED, 12)
    }));
    match (strict(), result) {
        (_, Ok((compared, known))) => {
            eprintln!("finding 1: {compared} compared, {known} with an irq_due-only difference");
            assert!(
                strict() || known == 1,
                "the finding no longer reproduces: tighten it"
            );
            assert!(!strict() || known == 0);
        }
        (true, Err(_)) => panic!("finding 1 stands: irq_due differs (NS4_FUZZ_STRICT=1)"),
        (false, Err(e)) => std::panic::resume_unwind(e),
    }
}

/// FINDING 2 (performance, `lower/guard.rs`, the `Window` row): after one interrupt has been
/// served, `irq.stall` stays non-zero for the rest of the run, so the entry guard refuses every
/// block and the engine runs the interpreter from then on. Exact, but the speed-up is lost for
/// the whole slice. W2B-SFIX is changing that row; strict mode asserts that blocks run again.
#[test]
fn finding_2_blocks_are_refused_for_good_after_the_first_interrupt() {
    let mut rng = Rng::new(7);
    let program = straight(&mut rng, &[Fam::Float, Fam::Fixed, Fam::Move], 60);
    let mut s = Scenario::new("finding-2", 7, program, 120);
    s.cfg = UopConfig {
        max_block_steps: 3,
        ..UopConfig::default()
    };
    s.irq = Some(Irq {
        at: vec![(10, 11)],
        isr: crate::core::uop::exec::h_flow::enc::rts(
            crate::core::uop::exec::h_flow::enc::TRUE,
            false,
            true,
        ),
    });
    let r = run(&s).unwrap();
    assert!(r.run.diff.is_none(), "{:?}", r.run.diff);
    assert!(r.taken > 0, "the interrupt was taken");
    let st = r.run.stats;
    let fast = st.steps_uop.saturating_sub(st.steps_interp_uop);
    eprintln!(
        "finding 2: {fast} fast steps of 120, window guard refused {} entries",
        st.guard_fail[1]
    );
    if strict() {
        assert!(
            fast > 60,
            "blocks must run again after the interrupt: {st:?}"
        );
    }
}

#[test]
fn the_census_replays_equal_on_both_uop_engines_and_the_firmware_entry_skips_cleanly() {
    use crate::core::census::Engine;
    use crate::core::uop::verify::census::replay;
    use crate::core::uop::verify::lockstep::{firmware_lockstep, Outcome};
    for e in [Engine::Uop, Engine::UopChecked] {
        let n = replay(e).unwrap_or_else(|d| panic!("{e:?}: {d}"));
        // EMPIRICAL: 1,580 hardware-measured cases (design 6.8: `census_cases.txt` plus
        // `census_extra_cases.txt`); the same count as `verify::gen::census` reads. Whether the
        // engine ran fast uops in them is not observable from here (`run_cases_on` builds its own
        // cores): a temporary probe in `core/census.rs` saw fast uops in all 3,160 runs.
        assert_eq!(n, 1580, "{e:?}: {n} census cases compared");
    }
    // without the user firmware the lockstep reports Skipped instead of failing
    let out = firmware_lockstep(256).expect("the firmware lockstep found no difference");
    assert!(
        matches!(out, Outcome::Skipped(_) | Outcome::Done { .. }),
        "{out:?}"
    );
}
