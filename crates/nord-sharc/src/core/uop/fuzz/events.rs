//! Events: interrupts scheduled at random cycles (also inside delay slots and loops), arithmetic
//! exception interrupts raised by the compute itself, `IDLE` woken by an interrupt, and the
//! interrupt-enable and mask registers written in the middle of a program.
//!
//! OWNER: W3-FUZZ. The event split (design 7.2) says a block entered at `steps` may run
//! `due - steps` steps unchecked and that a handler that may poke the controller lowers its
//! room; these tests put the deadline at every kind of place and compare the cores after the run
//! (the comparator reads `irq.due`, `irq.stall`, the latches, the serviced stack and the call
//! and loop stacks, so a drift of one step shows).
//!
//! KNOWN: after the first interrupt the `Window` row of the entry guard refuses every block
//! (`irq.stall` is cumulative, `findings.rs`), so the engine finishes the run through
//! `Core::step`: the results are still equal, but only the steps before the first interrupt run
//! as uops. The tests assert exactness and what ran before the interrupt, not after it;
//! [`after_interrupts`] records how much ran as uops and, with `NS4_FUZZ_STRICT=1` (once
//! W2B-SFIX has fixed the row), asserts that most of it did. Until then the service routine and
//! RTI as uops, an interrupt inside a delay slot or at a loop end, and the event split after a
//! served interrupt are untested at uop level.
use super::driver::{sweep, Totals};
use super::knobs::strict;
use super::scenario::{Irq, Scenario};
use super::shapes::{control, loops, straight, ALL, REGS};
use super::sieve::Fam;
use super::state::Prep;
use crate::core::uop::config::UopConfig;
use crate::core::uop::exec::h_flow::enc::{self, TRUE};
use crate::core::uop::verify::gen::Rng;

/// Cases per test at the CI bound. ASSUMED: 30 per test keep the file near 2 s in debug.
const CASES: u64 = 30;

/// Vectors an interrupt may be scheduled on: IRPTL bits 4 to 16 (`vectors.rs`), the first of
/// which (`TMZHI`) also pushes the status stack. VERIFIED: PRM Table A-12 as `vectors::slot`.
const VECTORS: [u8; 13] = [4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

/// A service routine of 1 to 3 fast instructions ending in `RTI` (16 or 48 bits).
fn isr(rng: &mut Rng, fams: &[Fam]) -> Vec<u16> {
    let n = 1 + rng.below(3) as usize;
    let mut p = straight(rng, fams, n);
    p.extend(if rng.below(2) == 0 {
        enc::rts(TRUE, false, true)
    } else {
        enc::ret_wide(TRUE, false, true)
    });
    p
}

/// `n` interrupts scheduled in `8..=horizon` cycles on random vectors. The first cycles are left
/// alone on purpose: the entry guard refuses a block that would run past the next deadline, so an
/// interrupt in the first cycles keeps the engine out of its blocks until it has been served.
fn schedule(rng: &mut Rng, n: u32, horizon: u64) -> Vec<(u64, u8)> {
    (0..n)
        .map(|_| {
            (
                8 + u64::from(rng.below(horizon as u32 - 7)),
                rng.pick(&VECTORS),
            )
        })
        .collect()
}

/// Records the steps the engine ran as fast uops against the steps it handed to `Core::step`, over
/// a sweep whose cases serve interrupts. Strict mode asserts that the fast steps are the majority
/// (finding 2 of `findings.rs`: after the first interrupt they are not).
#[track_caller]
fn after_interrupts(name: &str, t: &Totals) {
    let fast = t.steps_uop.saturating_sub(t.steps_interp_uop);
    eprintln!(
        "{name}: {fast} fast steps against {} through Core::step ({} interrupts in {} cases)",
        t.steps_core_step, t.taken, t.cases
    );
    if strict() {
        assert!(
            fast > t.steps_core_step,
            "{name}: most steps still run through Core::step after an interrupt: {t:?}"
        );
    }
}

/// A config whose blocks are short (2 to 5 steps), so some fit before the first deadline.
fn irq_cfg(rng: &mut Rng) -> UopConfig {
    UopConfig {
        max_block_steps: rng.pick(&[2, 3, 5]),
        ..UopConfig::default()
    }
}

/// The state of an event case: stale extensions, hidden state half of the time, a drawn PEy mode.
fn prep(rng: &mut Rng, fams: &[Fam]) -> Prep {
    Prep {
        simd: rng.below(2) == 1,
        stale: true,
        hidden: rng.below(2) == 1,
        dag: fams.contains(&Fam::Move),
        ..Prep::default()
    }
}

#[test]
fn interrupts_at_random_cycles_in_branching_code_equal_the_interpreter() {
    let t = sweep("irq-control", 30, CASES, |rng, seed| {
        let (p, starts) = {
            let n = 8 + rng.below(10) as usize;
            control(rng, &REGS, n)
        };
        let steps = 40 + u64::from(rng.below(80));
        let mut s = Scenario::new("irq-control", seed, p, steps);
        s.prep = prep(rng, &REGS);
        s.cfg = irq_cfg(rng);
        s.heavy = true;
        s.targets = starts;
        s.irq = Some(Irq {
            at: {
                let k = 1 + rng.below(3);
                schedule(rng, k, steps)
            },
            isr: isr(rng, &REGS),
        });
        s
    });
    assert!(t.taken >= t.cases / 2, "too few interrupts taken: {t:?}");
    after_interrupts("irq-control", &t);
}

#[test]
fn interrupts_inside_loops_and_straight_code_equal_the_interpreter() {
    let t = sweep("irq-loops", 31, CASES, |rng, seed| {
        let fams: &[Fam] = if rng.below(2) == 0 { &REGS } else { &ALL };
        let p = if rng.below(2) == 0 {
            loops(rng, fams)
        } else {
            {
                let n = 6 + rng.below(20) as usize;
                straight(rng, fams, n)
            }
        };
        let steps = 30 + u64::from(rng.below(60));
        let mut s = Scenario::new("irq-loops", seed, p, steps);
        s.prep = prep(rng, fams);
        s.cfg = irq_cfg(rng);
        s.heavy = true;
        s.irq = Some(Irq {
            at: {
                let k = 1 + rng.below(2);
                schedule(rng, k, steps)
            },
            isr: isr(rng, &REGS),
        });
        s
    });
    assert!(t.taken >= t.cases / 2, "too few interrupts taken: {t:?}");
    after_interrupts("irq-loops", &t);
}

#[test]
fn arithmetic_exception_interrupts_raised_by_the_compute_equal_the_interpreter() {
    // every interrupt unmasked and enabled, no interrupt scheduled: the float and fixed-point
    // overflow, underflow and invalid exceptions latch themselves inside the blocks (a block that
    // can raise one has the `Arith` guard class) and the interrupt is taken after the uop
    let t = sweep("irq-arith", 32, CASES, |rng, seed| {
        let n = 8 + rng.below(20) as usize;
        let mut s = Scenario::new(
            "irq-arith",
            seed,
            straight(
                rng,
                &[
                    Fam::Float,
                    Fam::Fixed,
                    Fam::Move,
                    Fam::Move,
                    Fam::Move,
                    Fam::Move,
                    Fam::Move,
                    Fam::Move,
                ],
                n,
            ),
            n as u64 + 10,
        );
        s.prep = Prep {
            simd: rng.below(2) == 1,
            stale: true,
            hidden: true,
            mr_edges: rng.below(2) == 1,
            ..Prep::default()
        };
        s.cfg = irq_cfg(rng);
        s.heavy = true;
        s.irq = Some(Irq {
            at: Vec::new(),
            isr: isr(rng, &REGS),
        });
        s
    });
    assert!(t.taken > 0, "no exception interrupt was ever taken: {t:?}");
}

#[test]
fn idle_woken_by_an_interrupt_equals_the_interpreter() {
    // a core without peripheral models runs IDLE as a uop (with a model it hands the step to the
    // interpreter, which the other tests cover through the access log)
    let t = sweep("irq-idle", 33, CASES, |rng, seed| {
        let mut p = {
            let n = 3 + rng.below(4) as usize;
            straight(rng, &REGS, n)
        };
        p.extend(enc::idle(rng.below(2) == 0));
        p.extend({
            let n = 4 + rng.below(4) as usize;
            straight(rng, &REGS, n)
        });
        let steps = 30;
        let mut s = Scenario::new("irq-idle", seed, p, steps);
        s.prep = prep(rng, &REGS);
        s.cfg = irq_cfg(rng);
        s.heavy = true;
        s.log = false;
        s.irq = Some(Irq {
            at: vec![(8 + u64::from(rng.below(12)), rng.pick(&VECTORS))],
            isr: isr(rng, &REGS),
        });
        s
    });
    assert!(
        t.taken >= t.cases / 2,
        "the sleeping core was not woken: {t:?}"
    );
}

#[test]
fn a_mask_or_mode_write_in_the_middle_of_a_program_takes_effect_at_the_same_step() {
    // IMASK and MODE1 loads (system registers) between fast instructions, with an interrupt
    // already latched: the load ends the block, the next entry sees the deadline at 0
    let t = sweep("irq-sysload", 34, CASES, |rng, seed| {
        let mut p = straight(rng, &REGS, 3);
        p.extend(enc::load_imm(0x7B, 0xFFFF_F000 & rng.next_u32())); // IMASK
        p.extend(straight(rng, &REGS, 3));
        p.extend(enc::load_imm(0x72, rng.below(2) << 12 | rng.below(2) << 21)); // MODE1: IRPTEN, PEYEN
        p.extend(straight(rng, &REGS, 6));
        let steps = 24;
        let mut s = Scenario::new("irq-sysload", seed, p, steps);
        s.prep = prep(rng, &REGS);
        s.cfg = irq_cfg(rng);
        s.heavy = true;
        s.irq = Some(Irq {
            at: schedule(rng, 2, 16),
            isr: isr(rng, &REGS),
        });
        s
    });
    assert!(t.steps_uop > 0, "{t:?}");
}
