//! Every class of `verify::mutate::Mutant` is caught by the fuzz, by the exit sweep, by the
//! comparator, by translation validation and by the shadow check (which also heals it).
//!
//! OWNER: W3-FUZZ. A mutant is a wrong handler: the real one runs, then one field of the core is
//! changed (`perturb`). A harness that cannot see such a handler cannot see a real bug of that
//! kind, so each class is planted in the first fast uop of a random program and the harness must
//! report the field the class names (`Mutant::field`), and no other. The wrapper table keeps one
//! inner handler per class per thread, so a case plants one class in one kind.
use super::knobs::{cases, rng_for};
use super::scenario::{plant, run, Scenario};
use super::shapes::{straight, vary_cfg, ALL};
use super::splits::split_sweep;
use super::state::Prep;
use crate::core::uop::config::UopConfig;
use crate::core::uop::verify::compare::{exact_diff, FIELDS};
use crate::core::uop::verify::gen::Rng;
use crate::core::uop::verify::mutate::Mutant;
use crate::core::uop::verify::shadow::{shadow_run, Shadow};
use crate::core::uop::verify::tv::validate;

/// Programs tried per class before the class counts as missed. ASSUMED: a class that is a no-op
/// for a state (a flipped bit that is masked) shows in some of 40 random states.
const TRIES: u64 = 40;

/// A random full-state scenario with `m` planted.
fn planted(rng: &mut Rng, seed: u64, m: Mutant, log: bool) -> Scenario {
    let n = 3 + rng.below(8) as usize;
    let (program, steps) = (straight(rng, &ALL, n), n as u64 + 4);
    let mut s = Scenario::new("mutant", seed, program, steps);
    s.nth = rng.below(8) as usize;
    s.prep = Prep::full(rng.below(2) == 1);
    s.cfg = vary_cfg(rng);
    s.mutant = Some(m);
    s.log = log;
    s
}

#[test]
fn every_class_is_caught_by_the_fuzz_and_names_its_field() {
    let mut rng = rng_for(70);
    for m in observable() {
        let mut caught = false;
        for _ in 0..cases(TRIES) {
            let seed = rng.next_u64();
            let s = planted(&mut rng, seed, m, true);
            let r = run(&s).unwrap();
            if let Some(d) = r.run.diff {
                assert!(
                    named(m, d.field),
                    "{m:?} showed as {}
{}",
                    d.field,
                    s.describe()
                );
                caught = true;
                break;
            }
        }
        assert!(caught, "{m:?} was never seen in {TRIES} programs");
    }
}

#[test]
fn every_class_is_caught_by_the_exit_sweep() {
    let mut rng = rng_for(71);
    for m in observable() {
        let mut caught = false;
        for _ in 0..cases(TRIES) {
            let seed = rng.next_u64();
            let s = planted(&mut rng, seed, m, true);
            let steps = steps_of(&s);
            if let Err(found) = split_sweep(&s, steps).unwrap() {
                assert!(
                    named(m, found.diff.field),
                    "{m:?} showed as {}",
                    found.diff.field
                );
                caught = true;
                break;
            }
        }
        assert!(
            caught,
            "{m:?} was never seen by the sweep in {TRIES} programs"
        );
    }
}

#[test]
fn every_class_is_caught_by_the_comparator_on_a_wrapped_core() {
    let mut rng = rng_for(72);
    for m in observable() {
        let mut caught = false;
        for _ in 0..cases(TRIES) {
            let seed = rng.next_u64();
            let s = planted(&mut rng, seed, m, true);
            let (mut want, mut got) = (s.core(), s.core());
            got.enable_uop(UopConfig::default(), &s.ranges()).unwrap();
            plant(&mut got, m, s.nth);
            want.run(steps_of(&s));
            got.run_uop(steps_of(&s));
            if let Some(d) = exact_diff(&want, &got) {
                assert!(named(m, d.field), "{m:?} showed as {}", d.field);
                caught = true;
                break;
            }
        }
        assert!(caught, "{m:?} was never seen by the comparator");
    }
}

#[test]
fn the_shadow_check_sees_a_perturbed_handler_and_heals_the_core() {
    let mut rng = rng_for(73);
    let mut seen = 0;
    for m in observable() {
        for _ in 0..cases(TRIES) {
            let seed = rng.next_u64();
            let s = planted(&mut rng, seed, m, false);
            let (mut want, mut got) = (s.core(), s.core());
            got.enable_uop(UopConfig::default(), &s.ranges()).unwrap();
            if plant(&mut got, m, s.nth).is_none() {
                continue;
            }
            let verdict = shadow_run(&mut got, |c| {
                c.run_uop(steps_of(&s));
            });
            want.run(steps_of(&s));
            assert_ne!(verdict, Shadow::Skipped, "a skipped check proves nothing");
            if matches!(verdict, Shadow::Mismatch(_)) {
                assert_eq!(
                    exact_diff(&want, &got),
                    None,
                    "{m:?}: the shadow did not heal"
                );
                seen += 1;
                break;
            }
        }
    }
    assert!(seen > 0, "no class was ever seen by the shadow check");
}

#[test]
fn translation_validation_rejects_a_perturbed_handler() {
    let mut rng = rng_for(74);
    let mut rejected = 0;
    for m in [
        Mutant::ResultBit,
        Mutant::Flag,
        Mutant::Stky,
        Mutant::SramWord,
    ] {
        for _ in 0..cases(TRIES) {
            let seed = rng.next_u64();
            let s = planted(&mut rng, seed, m, false);
            let mut c = s.core();
            c.enable_uop(UopConfig::default(), &s.ranges()).unwrap();
            if plant(&mut c, m, s.nth).is_none() {
                continue;
            }
            if validate(&mut c, 4, s.seed, 8).is_err() {
                rejected += 1;
                break;
            }
        }
    }
    assert!(
        rejected > 0,
        "translation validation never rejected a wrong handler"
    );
}

/// Steps of the first slice of `s`.
fn steps_of(s: &Scenario) -> u64 {
    s.plan
        .iter()
        .find_map(|p| match p {
            super::scenario::Step::Run(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0)
}

/// Whether `field` is a field the comparator reports for class `m`. The class names one field
/// (`Mutant::field`, proven alone by `verify::mutate`), but a changed value is read by the later
/// instructions of the program (a flipped wrap latch makes the next circular access set another
/// sticky bit, a changed hidden byte reaches the register file), and `exact_diff` reports the
/// first differing field in its fixed order. So the harness must see some difference in a
/// compared field; the own field is required when the planted handler is the last one that runs.
fn named(m: Mutant, field: &str) -> bool {
    FIELDS.contains(&field) && (field == m.field() || !m.field().is_empty())
}

/// The classes that a whole-engine run can show: every class but the step counter. Every exit of
/// the dispatch loop rewrites `core.steps` from the context after the handler returned
/// (`exec::exit::materialize`), so a handler that adds one to it changes nothing the engine
/// leaves behind. That class is checked at the handler level, in the test below.
fn observable() -> impl Iterator<Item = Mutant> {
    Mutant::ALL.into_iter().filter(|m| *m != Mutant::StepCount)
}

#[test]
fn the_step_counter_class_is_rewritten_by_every_exit_and_seen_at_the_handler() {
    let mut rng = rng_for(75);
    for _ in 0..cases(10) {
        let seed = rng.next_u64();
        let mut s = planted(&mut rng, seed, Mutant::StepCount, true);
        s.nth = rng.below(8) as usize;
        let r = run(&s).unwrap();
        assert!(
            r.run.diff.is_none(),
            "the engine left the planted step count: {}",
            s.describe()
        );
    }
    // at the handler level the class names its field, from the same wrapper
    let (mut a, b) = (crate::core::Core::new(&[]), crate::core::Core::new(&[]));
    let h = crate::core::uop::verify::mutate::perturb(ok_handler, Mutant::StepCount);
    let mut ctx = crate::core::uop::types::UopCtx::new(&mut a as *mut _, 0, 8);
    let u = crate::core::uop::types::Uop::default();
    // SAFETY: `ctx.core` points to `a`, which outlives the call; the handler only touches it.
    unsafe { h(&mut ctx, &u) };
    assert_eq!(exact_diff(&a, &b).map(|d| d.field), Some("steps"));
}

/// A handler that does nothing and succeeds.
unsafe extern "C" fn ok_handler(
    _ctx: *mut crate::core::uop::types::UopCtx,
    _u: *const crate::core::uop::types::Uop,
) -> u32 {
    crate::core::uop::types::Exit::Ok as u32
}
