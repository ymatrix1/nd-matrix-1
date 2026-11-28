//! Random straight programs per family: float, fixed point, moves (circular buffers included),
//! and all three mixed, on states with every kind of hidden state, PEy on and off, with the block
//! limit drawn per case so the cuts fall at different places.
//!
//! OWNER: W3-FUZZ. Each program is `n` fast instructions drawn from the pool of its family
//! (`sieve.rs`: census plus random encodings the family takes), run for `n` steps. A case that
//! the lowering did not turn into fast uops fails as vacuous (`driver::sweep`).
use super::driver::sweep;
use super::scenario::Scenario;
use super::shapes::{cb_insn, straight, vary_cfg, ALL};
use super::sieve::Fam;
use super::state::Prep;
use crate::core::uop::verify::gen::Rng;

/// Cases per test at the CI bound. ASSUMED: 40 programs of up to 24 instructions per family
/// keep this file near 2 s in the debug profile; a long run multiplies it through the knob.
const CASES: u64 = 40;

/// A start state for a case of `fams`: the memory and DAG fill only where a family reads memory,
/// the stale float extension wherever a float register moves, the rest drawn.
fn prep(rng: &mut Rng, fams: &[Fam]) -> Prep {
    Prep {
        simd: rng.below(2) == 1,
        stale: fams.contains(&Fam::Float) || fams.contains(&Fam::Move) || rng.below(2) == 0,
        hidden: rng.below(2) == 1,
        dag: fams.contains(&Fam::Move),
        into_code: false,
        mr_edges: fams.contains(&Fam::Fixed) && rng.below(2) == 0,
        stack: 0,
    }
}

/// The generator of a straight case over `fams`.
fn case(name: &'static str, fams: &'static [Fam]) -> impl FnMut(&mut Rng, u64) -> Scenario {
    move |rng, seed| {
        let n = 3 + rng.below(22) as usize;
        let mut s = Scenario::new(name, seed, straight(rng, fams, n), n as u64);
        s.prep = prep(rng, fams);
        s.cfg = vary_cfg(rng);
        s
    }
}

#[test]
fn float_programs_equal_the_interpreter() {
    let t = sweep("float", 1, CASES, case("float", &[Fam::Float]));
    assert!(t.steps_uop > t.steps_interp_uop, "{t:?}");
}

#[test]
fn fixed_point_programs_equal_the_interpreter() {
    let t = sweep("fixed", 2, CASES, case("fixed", &[Fam::Fixed]));
    assert!(t.steps_uop > t.steps_interp_uop, "{t:?}");
}

#[test]
fn move_programs_equal_the_interpreter() {
    let t = sweep("moves", 3, CASES, case("moves", &[Fam::Move]));
    assert!(t.steps_uop > t.steps_interp_uop, "{t:?}");
}

#[test]
fn mixed_programs_on_the_busiest_state_equal_the_interpreter() {
    sweep("mixed", 4, CASES, |rng, seed| {
        let n = 2 + rng.below(30) as usize;
        let mut s = Scenario::new("mixed", seed, straight(rng, &ALL, n), n as u64);
        s.prep = Prep::full(rng.below(2) == 1);
        s.cfg = vary_cfg(rng);
        s.heavy = true;
        s
    });
}

#[test]
fn circular_buffer_moves_through_i7_and_i15_equal_the_interpreter() {
    // the wrap latch: the DAG fill leaves I7 or I15 at the end of a four-word buffer, and half of
    // the instructions are moves whose uop latches the wrap
    sweep("circular", 5, CASES, |rng, seed| {
        let n = 3 + rng.below(16) as usize;
        let mut p = Vec::new();
        for _ in 0..n {
            p.extend(if rng.below(2) == 0 {
                cb_insn(rng)
            } else {
                straight(rng, &[Fam::Move], 1)
            });
        }
        let mut s = Scenario::new("circular", seed, p, n as u64);
        s.prep = Prep {
            dag: true,
            stale: true,
            simd: rng.below(2) == 1,
            ..Prep::default()
        };
        s.cfg = vary_cfg(rng);
        s
    });
}

#[test]
fn the_same_program_is_exact_with_pey_on_and_off_and_at_every_block_limit() {
    for simd in [false, true] {
        sweep("pey", 6 + u64::from(simd), CASES / 2, move |rng, seed| {
            let n = 4 + rng.below(12) as usize;
            let mut s = Scenario::new("pey", seed, straight(rng, &ALL, n), n as u64);
            s.prep = Prep::full(simd);
            s.cfg = vary_cfg(rng);
            s
        });
    }
}

#[test]
fn multiplier_edges_and_saturation_modes_equal_the_interpreter() {
    // MR at the edges of the 80-bit range with fixed-point multiply-accumulate and the saturating
    // and rounding forms: the MR spill and the ALUSAT clamp
    sweep("mr-sat", 8, CASES, |rng, seed| {
        let n = 3 + rng.below(13) as usize;
        let mut s = Scenario::new("mr-sat", seed, straight(rng, &[Fam::Fixed], n), n as u64);
        s.prep = Prep {
            mr_edges: true,
            hidden: true,
            simd: rng.below(2) == 1,
            ..Prep::default()
        };
        s.heavy = true;
        s.cfg = vary_cfg(rng);
        s
    });
}
