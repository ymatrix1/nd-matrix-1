//! The async translation path: code changes, then `uop_maintain_async` submits the dropped
//! ranges to a translation service, the answers are installed, and the run goes on. The service is
//! synchronous (pumped on the calling thread through a scratch core) in one test and a real
//! worker thread in the other; the stop pc exercises the install rule of design P7 (a block that
//! holds the live stop pc as an interior uop is not installed from the worker's answer).
//!
//! OWNER: W3-FUZZ. What must hold: whatever the worker returns, whenever it returns it, the state
//! after every slice equals the interpreter's, because a block the worker built from words that
//! have since changed is refused by its `code_hash`, and a block that is installed equals the
//! interpreter by the same argument as a block translated on the spot.
use super::driver::sweep;
use super::scenario::{Scenario, Step};
use super::shapes::{control, loops, vary_cfg, REGS};
use super::state::Prep;
use crate::core::uop::verify::gen::{Rng, CODE_PC};

/// Cases per test at the CI bound. ASSUMED: 20 keeps the file near 2 s (a threaded case spawns a
/// worker thread and a scratch core).
const CASES: u64 = 20;

/// A program, its instruction starts and a plan of slices with code changes and async rounds.
fn changing(rng: &mut Rng, seed: u64, name: &'static str) -> Scenario {
    let (p, starts) = if rng.below(2) == 0 {
        {
            let n = 8 + rng.below(8) as usize;
            control(rng, &REGS, n)
        }
    } else {
        (loops(rng, &REGS), vec![CODE_PC])
    };
    let mut plan = vec![Step::Run(10 + u64::from(rng.below(20)))];
    for _ in 0..2 + rng.below(3) {
        let at = starts[rng.below(starts.len() as u32) as usize];
        plan.push(Step::Poke {
            word: at / 2,
            value: if rng.below(2) == 0 {
                0x0001_0001
            } else {
                rng.next_u32()
            },
        });
        plan.push(Step::MaintainAsync);
        plan.push(Step::Run(8 + u64::from(rng.below(30))));
    }
    let mut s = Scenario::new(name, seed, p, 0);
    s.plan = plan;
    s.prep = Prep {
        stale: true,
        simd: rng.below(2) == 1,
        ..Prep::default()
    };
    s.cfg = vary_cfg(rng);
    s.targets = starts;
    s
}

#[test]
fn rounds_through_a_synchronous_service_equal_the_interpreter() {
    let t = sweep("service-sync", 50, CASES, |rng, seed| {
        changing(rng, seed, "service-sync")
    });
    assert!(t.maintain >= t.cases, "every case ran its rounds: {t:?}");
}

#[test]
fn rounds_through_a_worker_thread_equal_the_interpreter() {
    let t = sweep("service-thread", 51, CASES / 2, |rng, seed| {
        let mut s = changing(rng, seed, "service-thread");
        s.threaded = true;
        s
    });
    assert!(t.maintain >= t.cases, "every case ran its rounds: {t:?}");
}

#[test]
fn a_stop_pc_inside_the_program_stays_a_block_start_through_the_service() {
    // the pass idle pc is a block start for the translator at arming, for the worker (fixed at
    // spawn) and for the retranslation; a block the worker built across it is not installed
    sweep("service-stop", 52, CASES, |rng, seed| {
        let mut s = changing(rng, seed, "service-stop");
        s.threaded = rng.below(2) == 0;
        s.stop_pc = Some(s.targets[rng.below(s.targets.len() as u32) as usize]);
        s
    });
}
