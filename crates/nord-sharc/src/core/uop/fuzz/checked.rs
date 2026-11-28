//! The pass loops and the engine-level checks on real families: `lean_pass_uop` against
//! `lean_pass`, the checked pass (`Engine::UopChecked`), the shadow check and translation
//! validation, all on random programs whose instructions the lowering turns into fast uops.
//!
//! OWNER: W3-FUZZ. A pass is a program that ends in the poll loop `IF TRUE JUMP start` at the idle
//! pc; it starts there (the trigger write, then the jump, then the body, back at the idle pc).
//! The pass runner functions are the ones `lle_sharc_bench` and `uop-report` use
//! (`runner::uop_pass`), so what is checked here is what the tools run. The cores have no
//! peripheral model (no access log): the checked and shadow forms rewind the core and skip a core
//! with peripherals, so a verdict of `Skipped` would prove nothing and is refused.
use super::knobs::{cases, rng_for, strict};
use super::shapes::{loops, straight, ALL, REGS};
use super::sieve::Fam;
use super::state::{prepare, Prep};
use crate::core::support::CODE;
use crate::core::uop::config::UopConfig;
use crate::core::uop::exec::h_flow::enc::{self, TRUE};
use crate::core::uop::verify::compare::exact_diff;
use crate::core::uop::verify::gen::{core_with_code, fill_state, Rng};
use crate::core::uop::verify::shadow::{shadow_run, Shadow};
use crate::core::uop::verify::snap::CoreSnap;
use crate::core::uop::verify::tv::validate;
use crate::core::Core;
use crate::runner::pass::PassConfig;
use crate::runner::system::lean_pass;
use crate::runner::uop_pass::{
    arm_uop, lean_pass_uop, lean_pass_uop_checked, lean_pass_uop_shadowed,
};

/// Mailbox word of the passes: a scripted device word that always reads 0, so a pass ends
/// whenever the pc is back on the idle instruction (the poll program of `uop_pass::support`).
const TRIGGER: u32 = 0x3000_0000;

/// Cases per test at the CI bound. ASSUMED: 15 passes of three rounds keep the file near 2 s.
const CASES: u64 = 15;

/// A core idling at the end of `body`, which is followed by the poll jump to its own start.
pub(super) fn pass_core(body: &[u16], seed: u64, simd: bool) -> (Core, PassConfig) {
    let idle_pc = CODE + body.len() as u32;
    let mut program = body.to_vec();
    program.extend(enc::jump(TRUE, CODE, false, false));
    let mut c = core_with_code(&program);
    c.preset_device(TRIGGER, 0);
    prepare(
        &mut c,
        seed,
        &Prep {
            stale: true,
            hidden: seed.is_multiple_of(2),
            simd,
            ..Prep::default()
        },
    );
    c.irq.due = u64::MAX;
    c.pc = idle_pc;
    let cfg = PassConfig {
        trigger: TRIGGER,
        idle_pc,
        max_steps: 300,
        irq: None,
    };
    (c, cfg)
}

/// The body of a case: straight code of all families, or a loop nest, drawn.
fn body(rng: &mut Rng) -> Vec<u16> {
    if rng.below(3) == 0 {
        loops(rng, &REGS)
    } else {
        let n = 4 + rng.below(20) as usize;
        straight(rng, &ALL, n)
    }
}

/// Steps the engine ran through fast uops, and the check that it ran some.
fn fast_steps(c: &Core) -> u64 {
    let s = c.uop_stats();
    s.steps_uop.saturating_sub(s.steps_interp_uop)
}

#[test]
fn passes_under_the_engine_equal_passes_under_the_interpreter() {
    let mut rng = rng_for(60);
    let mut fast = 0;
    for n in 0..cases(CASES) {
        let (seed, simd) = (rng.next_u64(), rng.below(2) == 1);
        let program = body(&mut Rng::new(seed));
        let ((mut want, cfg), (mut got, _)) = (
            pass_core(&program, seed, simd),
            pass_core(&program, seed, simd),
        );
        arm_uop(&mut got, UopConfig::default(), cfg.idle_pc).unwrap();
        for round in 0..3 {
            let (a, b) = (lean_pass(&mut want, &cfg), lean_pass_uop(&mut got, &cfg));
            assert_eq!(a, b, "case {n} seed {seed:#x} round {round}: (steps, idle)");
            let d = exact_diff(&want, &got);
            assert!(
                d.is_none(),
                "case {n} seed {seed:#x} round {round}: {d:?}\n{program:04x?}"
            );
        }
        assert!(
            fast_steps(&got) > 0,
            "case {n} seed {seed:#x}: nothing ran fast: {:?}",
            got.uop_stats()
        );
        fast += fast_steps(&got);
    }
    assert!(fast > 0);
}

#[test]
fn the_checked_and_shadowed_passes_find_no_difference_and_really_check() {
    let mut rng = rng_for(61);
    for n in 0..cases(CASES) {
        let (seed, simd) = (rng.next_u64(), rng.below(2) == 1);
        let program = body(&mut Rng::new(seed));
        let (mut want, cfg) = pass_core(&program, seed, simd);
        let (mut shadowed, _) = pass_core(&program, seed, simd);
        let (mut checked, _) = pass_core(&program, seed, simd);
        for c in [&mut shadowed, &mut checked] {
            arm_uop(c, UopConfig::default(), cfg.idle_pc).unwrap();
        }
        for round in 0..2 {
            let w = lean_pass(&mut want, &cfg);
            let (steps, idle, verdict) = lean_pass_uop_shadowed(&mut shadowed, &cfg);
            assert_eq!(
                verdict,
                Shadow::Match,
                "case {n} seed {seed:#x} round {round}: a skipped check proves nothing\n{program:04x?}"
            );
            assert_eq!((steps, idle), w, "case {n} round {round}");
            let (steps, idle, diff) = lean_pass_uop_checked(&mut checked, &cfg);
            assert_eq!(
                (steps, idle, diff),
                (w.0, w.1, None),
                "case {n} round {round}"
            );
            assert_eq!(exact_diff(&want, &shadowed), None, "case {n} round {round}");
            assert_eq!(exact_diff(&want, &checked), None, "case {n} round {round}");
        }
        assert!(
            fast_steps(&shadowed) > 0 && fast_steps(&checked) > 0,
            "case {n}: vacuous check"
        );
    }
}

/// What `tv::validate` does, with the one known difference counted instead of stopping the run:
/// each random state is run by the engine and replayed by the interpreter, and a difference in a
/// field other than `irq_due` panics with the state number. Returns `(compared, irq_due_only)`.
/// FINDING (`findings.rs`): `irq_due` alone differs after an exception-raising fixed-point
/// instruction on some random states; `NS4_FUZZ_STRICT=1` makes that a failure too.
pub(super) fn random_states(c: &mut Core, states: u32, seed: u64, steps: u64) -> (u32, u32) {
    let start = CoreSnap::capture(c);
    let simd = c.simd();
    let mut rng = Rng::new(seed);
    let (mut compared, mut known) = (0, 0);
    for n in 0..states {
        start.restore(c);
        fill_state(c, &mut rng, simd);
        match shadow_run(c, |c| {
            c.run_uop(steps);
        }) {
            Shadow::Match => compared += 1,
            Shadow::Mismatch(d) if d.field == "irq_due" && !strict() => known += 1,
            Shadow::Mismatch(d) => panic!("state {n} of seed {seed:#x}: {d:?}"),
            Shadow::Skipped => panic!("a skipped check proves nothing (state {n})"),
        }
    }
    start.restore(c);
    (compared, known)
}

#[test]
fn translation_validation_on_random_states_runs_the_real_families_and_matches() {
    let mut rng = rng_for(62);
    let mut compared = 0;
    for n in 0..cases(CASES) {
        let (seed, simd) = (rng.next_u64(), rng.below(2) == 1);
        let program = body(&mut Rng::new(seed));
        let (mut c, cfg) = pass_core(&program, seed, simd);
        c.pc = CODE; // validation runs the body from its start
        arm_uop(&mut c, UopConfig::default(), cfg.idle_pc).unwrap();
        let (done, known) = random_states(&mut c, 6, seed, 12);
        compared += done;
        assert_eq!(done + known, 6, "case {n} seed {seed:#x}");
        assert!(
            fast_steps(&c) > 0,
            "case {n}: the validation ran nothing fast: {:?}",
            c.uop_stats()
        );
    }
    assert!(compared > 0);
}

#[test]
fn the_library_validator_accepts_programs_without_exceptions() {
    // moves and the multiplier-free loop bodies cannot raise an arithmetic exception, so the whole
    // comparison, `irq_due` included, is `tv::validate` as the tools run it
    let mut rng = rng_for(64);
    for n in 0..cases(CASES) {
        let seed = rng.next_u64();
        let len = 4 + rng.below(12) as usize;
        let program = straight(&mut Rng::new(seed), &[Fam::Move], len);
        let (mut c, cfg) = pass_core(&program, seed, false);
        c.pc = CODE;
        arm_uop(&mut c, UopConfig::default(), cfg.idle_pc).unwrap();
        assert_eq!(
            validate(&mut c, 4, seed, 12),
            Ok(4),
            "case {n} seed {seed:#x}
{program:04x?}"
        );
    }
}

#[test]
fn a_shadow_run_of_the_engine_matches_and_ran_fast_uops() {
    let mut rng = rng_for(63);
    let mut fast = 0;
    for n in 0..cases(CASES) {
        let (seed, simd) = (rng.next_u64(), rng.below(2) == 1);
        let program = body(&mut Rng::new(seed));
        let (mut c, cfg) = pass_core(&program, seed, simd);
        c.pc = CODE;
        arm_uop(&mut c, UopConfig::default(), cfg.idle_pc).unwrap();
        let steps = 20 + u64::from(rng.below(40));
        let verdict = shadow_run(&mut c, |c| {
            c.run_uop(steps);
        });
        // FINDING 1 (`findings.rs`): an `irq_due`-only difference is counted, not failed
        let known = matches!(&verdict, Shadow::Mismatch(d) if d.field == "irq_due" && !strict());
        assert!(
            known || verdict == Shadow::Match,
            "case {n} seed {seed:#x}: {verdict:?}\n{program:04x?}"
        );
        fast += u64::from(fast_steps(&c) > 0);
    }
    // a short slice can end before its first block: at most one case in twenty
    assert!(
        fast * 20 + 20 >= cases(CASES) * 19,
        "only {fast} of {} cases ran fast uops",
        cases(CASES)
    );
}
