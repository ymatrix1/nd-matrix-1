//! Hand-built all-`Interp` blocks against the interpreter: the first acceptance row of W2-DRIVE.
//!
//! OWNER: W2-DRIVE. Every engine in this file is armed through `fixture::arm_interp`, whatever
//! the fast families do, so each test checks the driver, the dispatch loop, the exits and the
//! chaining on code the interpreter body runs verbatim. A test that did not run a block through a
//! uop would pass vacuously; each one asserts `exercised()` or the counter it relies on.
use crate::core::support::CODE;
use crate::core::uop::drive::fixture::{arm_interp, run_both, Setup};
use crate::core::uop::types::{Exit, NONE};
use crate::core::uop::verify::compare::exact_diff;
use crate::core::uop::verify::count_alloc::allocs_in;
use crate::core::uop::verify::fuzz::iterations;
use crate::core::uop::verify::gen::{core_with_code, fill_state, random_program, Rng, CODE_PC};

/// A loop: NOP, `R3 = R1 + R2`, NOP, `IF NE JUMP CODE`, two NOPs after it. With `R1 + R2` not
/// zero the jump is always taken; the pass is four steps.
const LOOP: [u16; 10] = [
    0x0001, 0x013e, 0x0000, 0x1312, 0x0001, 0x0620, 0x0012, 0x0000, 0x0001, 0x0001,
];

#[test]
fn all_interp_blocks_equal_the_interpreter_on_random_programs() {
    let mut rng = Rng::new(0x5eed_0001);
    let (mut blocks, mut programs) = (0, 0);
    for _ in 0..iterations() {
        let case = rng.next_u64();
        let mut r = Rng::new(case);
        let k = 2 + r.below(14) as usize;
        let program = random_program(&mut r, k);
        let simd = r.below(2) == 1;
        let (mut want, mut got) = (core_with_code(&program), core_with_code(&program));
        for c in [&mut want, &mut got] {
            fill_state(c, &mut Rng::new(case), simd);
        }
        arm_interp(&mut got, &[(CODE_PC, CODE_PC + program.len() as u32)]);
        let (fw, fg) = (want.run(k as u64), got.run_uop(k as u64));
        assert_eq!(fw, fg, "case {case:#x}: the fault");
        assert_eq!(exact_diff(&want, &got), None, "case {case:#x}");
        let s = got.uop_stats();
        assert_eq!(
            s.steps_uop, s.steps_interp_uop,
            "case {case:#x}: only Interp uops exist"
        );
        blocks += s.blocks_run;
        programs += 1;
    }
    assert!(
        blocks >= programs / 2,
        "blocks ran in most programs: {blocks} in {programs}"
    );
}

#[test]
fn a_taken_loop_branch_chains_through_the_block_and_links_it_to_itself() {
    let setup = Setup::program(&LOOP).with(|c| (c.r[1], c.r[2]) = (1, 2));
    let r = run_both(&setup, 100);
    assert_eq!(r.diff, None, "{r:?}");
    assert!(r.exercised(), "{r:?}");
    let s = r.stats;
    assert_eq!(s.steps_uop + s.steps_core_step, 100);
    assert_eq!(
        s.exits[Exit::Jump as usize],
        25,
        "one Jump per pass of four steps: {s:?}"
    );
    let mut c = setup.armed();
    assert_eq!(c.run_uop(100), None);
    let hot = c.uop.as_ref().unwrap();
    let (id, _) = hot.pcmap(false).get(CODE).unwrap();
    let link = hot.blocks[id as usize].links[1];
    assert_eq!(link.id, id, "the loop block's jump leads to itself");
    assert_eq!(link.epoch, hot.epoch);
}

#[test]
fn a_not_taken_branch_falls_through_to_the_next_block_and_the_loop_runs_out_of_code() {
    // R1 + R2 = 0: AZ is set, `IF NE JUMP` falls through to the two NOPs and then past the
    // program into zero parcels, which both engines treat alike
    let setup = Setup::program(&LOOP).with(|c| (c.r[1], c.r[2]) = (0, 0));
    let r = run_both(&setup, 12);
    assert_eq!(r.diff, None, "{r:?}");
    assert!(r.exercised(), "{r:?}");
    assert_eq!(r.stats.exits[Exit::Jump as usize], 0);
    assert_eq!(
        r.stats.steps_uop, 6,
        "the six instructions of the program: {r:?}"
    );
    assert_eq!(
        r.stats.steps_uop + r.stats.steps_core_step,
        12,
        "the rest is Core::step: {r:?}"
    );
}

#[test]
fn a_delayed_branch_and_its_slots_run_like_the_interpreter() {
    // NOP, JUMP (DB) CODE, two delay slots, then the loop closes itself
    let program = [0x0001, 0x063e, 0x0412, 0x0000, 0x0001, 0x0001];
    for steps in [1, 2, 3, 4, 5, 6, 7, 9, 20, 41] {
        let r = run_both(&Setup::program(&program), steps);
        assert_eq!(r.diff, None, "{steps} steps: {r:?}");
        if steps > 1 {
            assert!(r.exercised(), "{steps} steps: {r:?}");
        }
    }
}

#[test]
fn every_step_count_up_to_a_few_passes_ends_in_the_interpreters_state() {
    // an event-free loop run for 1..=50 steps: the limit cuts blocks at every offset
    for steps in 1..=50 {
        let setup = Setup::program(&LOOP).with(|c| (c.r[1], c.r[2]) = (3, 4));
        let r = run_both(&setup, steps);
        assert_eq!(r.diff, None, "{steps} steps: {r:?}");
        assert_eq!(
            r.stats.steps_uop + r.stats.steps_core_step,
            steps,
            "{steps} steps"
        );
    }
}

#[test]
fn a_stop_pc_inside_the_loop_ends_each_chain_there() {
    let setup = Setup::program(&LOOP)
        .with(|c| (c.r[1], c.r[2]) = (1, 2))
        .stop_pc(CODE + 4);
    let (mut want, mut got) = (setup.core(), setup.armed());
    assert_ne!(got.uop_run.stop_pc, NONE);
    // each call runs to the stop pc and returns there, like a pass that ends on the idle pc
    for pass in 0..6 {
        let before = got.steps;
        assert!(got.uop_try(u64::MAX), "pass {pass}");
        assert_eq!(got.pc, CODE + 4, "pass {pass}");
        // the interpreter reaches the same pc in the same number of steps
        while want.steps < got.steps {
            want.step().unwrap();
        }
        assert_eq!(want.pc, got.pc);
        assert!(got.steps > before);
    }
    assert_eq!(want.state_mismatch(&got), None);
}

#[test]
fn a_steady_uop_try_allocates_nothing() {
    // P14: a four-step loop run through chained blocks: link lookup, mid-block entry, `Jump`
    // exits, the `Interp` body and the exit accounting, once the decode cache and the links are
    // warm
    let setup = Setup::program(&LOOP).with(|c| (c.r[1], c.r[2]) = (1, 2));
    let mut c = setup.armed();
    for _ in 0..3 {
        let limit = c.steps + 40;
        assert!(c.uop_try(limit));
    }
    let before = c.uop_stats().steps_uop;
    let (all_ran, allocs) = allocs_in(|| {
        (0..8).all(|_| {
            let limit = c.steps + 40;
            c.uop_try(limit)
        })
    });
    assert!(all_ran);
    assert_eq!(allocs, 0, "a steady uop_try allocates");
    assert!(
        c.uop_stats().steps_uop >= before + 8 * 40,
        "{:?}",
        c.uop_stats()
    );
}
