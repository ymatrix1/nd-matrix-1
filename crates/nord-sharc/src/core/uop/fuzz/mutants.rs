//! Every class of `verify::mutate::Mutant` is caught by the fuzz, by the exit sweep, by the
//! comparator, by translation validation and by the shadow check (which also heals it).
//!
//! OWNER: W3-FUZZ. A mutant is a wrong handler: the real one runs, then one field of the core is
//! changed (`perturb`). A harness that cannot see such a handler cannot see a real bug of that
//! kind, so each class is planted in the first fast uop of a random program and the harness must
//! see it. The wrapper table keeps one inner handler per class per thread, so a case plants one
//! class in one kind. In a program of several instructions a changed value is read by the later
//! ones and the comparator reports the first differing field in its fixed order, so those tests
//! only require a compared field; the test with ONE fast instruction (the planted handler is then
//! the last that runs) requires exactly the field the class names (`Mutant::field`).
//!
//! `Mutant::StepCount` is not among the whole-engine classes: it adds to `core.steps`, which every
//! exit of the engine rewrites from the context, so it is dead in the engine. The class that the
//! engine can show is a handler that miscounts the context (`ctx.done`), planted below.
use super::knobs::{cases, rng_for};
use super::scenario::{plant, plant_with, run, Scenario};
use super::shapes::{straight, vary_cfg, ALL};
use super::splits::split_sweep;
use super::state::Prep;
use crate::core::uop::config::UopConfig;
use crate::core::uop::types::{Exit, Handler, Uop, UopCtx};
use crate::core::uop::verify::compare::{exact_diff, FIELDS};
use crate::core::uop::verify::gen::Rng;
use crate::core::uop::verify::mutate::Mutant;
use crate::core::uop::verify::shadow::{shadow_run, Shadow};
use crate::core::uop::verify::tv::validate;
use std::cell::Cell;

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
fn every_class_is_caught_by_the_fuzz_and_shows_a_compared_field() {
    let mut rng = rng_for(70);
    for m in observable() {
        let mut caught = false;
        for _ in 0..cases(TRIES) {
            let seed = rng.next_u64();
            let s = planted(&mut rng, seed, m, true);
            let r = run(&s).unwrap();
            if let Some(d) = r.run.diff {
                assert!(
                    compared(d.field),
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
                    compared(found.diff.field),
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
                assert!(compared(d.field), "{m:?} showed as {}", d.field);
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

/// Whether `field` is a field the comparator reports. A multi-instruction program cannot name the
/// planted class's own field (see the module header); `the_only_handler_that_runs_names_its_own_field`
/// requires that.
fn compared(field: &str) -> bool {
    FIELDS.contains(&field)
}

/// The classes that a whole-engine run can show: every class but the step counter (module header).
fn observable() -> impl Iterator<Item = Mutant> {
    Mutant::ALL.into_iter().filter(|m| *m != Mutant::StepCount)
}

/// One fast instruction and one step: the planted handler runs once and is the last
/// that runs, so nothing reads the value it changed (a later step reads `cb_wrap` into `irptl`).
fn single(rng: &mut Rng, seed: u64, m: Mutant) -> Scenario {
    let mut s = planted(rng, seed, m, true);
    s.program = straight(rng, &ALL, 1);
    s.plan = vec![super::scenario::Step::Run(1)];
    s.nth = 0;
    s
}

#[test]
fn the_only_handler_that_runs_names_its_own_field() {
    let mut rng = rng_for(76);
    for m in observable() {
        let (mut by_run, mut by_sweep) = (0, 0);
        for _ in 0..cases(TRIES) {
            let seed = rng.next_u64();
            let s = single(&mut rng, seed, m);
            let (mut want, mut got) = (s.core(), s.core());
            got.enable_uop(s.cfg.clone(), &s.ranges()).unwrap();
            if plant(&mut got, m, 0).is_none() {
                continue;
            }
            want.run(1);
            got.run_uop(1);
            if let Some(d) = exact_diff(&want, &got) {
                assert_eq!(d.field, m.field(), "{m:?} by run\n{}", s.describe());
                by_run += 1;
            }
            if let Err(found) = split_sweep(&s, 1).unwrap() {
                assert_eq!(found.diff.field, m.field(), "{m:?} by sweep");
                by_sweep += 1;
            }
        }
        assert!(by_run > 0, "{m:?} was never named by a run");
        assert!(by_sweep > 0, "{m:?} was never named by the sweep");
    }
}

thread_local! {
    /// The handler [`miscount`] wraps, per thread.
    static COUNTED: Cell<Option<Handler>> = const { Cell::new(None) };
}

/// A handler that runs the real one and then counts one step too many in the context.
unsafe extern "C" fn miscounting(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let exit = match COUNTED.with(Cell::get) {
        Some(h) => h(ctx, u),
        None => Exit::Slow as u32,
    };
    if exit == Exit::Ok as u32 {
        (*ctx).done += 1;
    }
    exit
}

fn miscount(h: Handler) -> Handler {
    COUNTED.with(|c| c.set(Some(h)));
    miscounting
}

#[test]
fn a_handler_that_miscounts_the_context_steps_is_seen_as_the_step_count() {
    // `Mutant::StepCount` writes `core.steps`, which every exit rewrites: dead in the engine
    let mut rng = rng_for(75);
    let mut dead = 0;
    for _ in 0..cases(10) {
        let seed = rng.next_u64();
        let s = planted(&mut rng, seed, Mutant::StepCount, true);
        let r = run(&s).unwrap();
        assert!(r.run.diff.is_none(), "{}", s.describe());
        dead += u64::from(r.exercised());
    }
    assert!(dead > 0, "the dead class was never run");
    // the engine's own step counter is `ctx.done`: a handler that gets it wrong is a real bug
    let mut seen = 0;
    for _ in 0..cases(TRIES) {
        let seed = rng.next_u64();
        let s = single(&mut rng, seed, Mutant::StepCount);
        let (mut want, mut got) = (s.core(), s.core());
        got.enable_uop(s.cfg.clone(), &s.ranges()).unwrap();
        if plant_with(&mut got, 0, miscount).is_none() {
            continue;
        }
        want.run(1);
        got.run_uop(1);
        if let Some(d) = exact_diff(&want, &got) {
            assert_eq!(d.field, "steps", "{}", s.describe());
            seen += 1;
        }
    }
    assert!(seen > 0, "a miscounted step was never seen");
    // at the handler level `Mutant::StepCount` names its field, from the same wrapper
    let (mut a, b) = (crate::core::Core::new(&[]), crate::core::Core::new(&[]));
    let h = crate::core::uop::verify::mutate::perturb(ok_handler, Mutant::StepCount);
    let mut ctx = UopCtx::new(&mut a as *mut _, 0, 8);
    let u = Uop::default();
    // SAFETY: `ctx.core` points to `a`, which outlives the call; the handler only touches it.
    unsafe { h(&mut ctx, &u) };
    assert_eq!(exact_diff(&a, &b).map(|d| d.field), Some("steps"));
}

/// A handler that does nothing and succeeds.
unsafe extern "C" fn ok_handler(_ctx: *mut UopCtx, _u: *const Uop) -> u32 {
    Exit::Ok as u32
}
