//! Measurement and rationale of the bail on an arithmetic exception (`run.rs`): how often a fixed
//! uop answers `Slow` because its effect sets a sticky bit, and why a bit that is already latched
//! does not change that.
//!
//! OWNER: W2-FIXED, tests. The interpreter does not latch a sticky bit once: `Core::step` calls
//! `note_arith_exception_in` for every event, which ORs the bit into STKY and then
//! `raise_interrupt`s the matching vector (`FIXI`, `FLTOI`, ...), and that sets the latch and
//! pokes `irq.due` to 0 whether or not the STKY bit was already set (PRM A-21: "every event latches
//! again"). The poke is observable (`exact_diff` compares `irq.due`) and makes the prologue of the
//! next step run the sequencer. So the latched case is not a no-op, and the handler bails on every
//! effect that sets a sticky bit.
use super::diff::Case;
use super::diff_sweep::{sweep, Tables};
use super::eff::STKY_AOS;
use super::fixture::{random_programs, t2a};
use super::kinds::ALU_OPS;
use crate::core::uop::types::Exit;
use crate::core::uop::verify::gen::{census_encodings, Rng};
use crate::core::Core;
use crate::visa::Kind;

/// The share of the census sweep (`census_instructions_equal_the_interpreter`, 2 seeds, both PEy
/// modes) on which the registered handler bailed, and how many of those bails raised only bits
/// that STKY already held.
#[derive(Debug, Default)]
struct Share {
    cases: usize,
    bails: usize,
    already_latched: usize,
}

fn measure(programs: &[Vec<u16>]) -> Share {
    let mut s = Share::default();
    sweep(programs, 2, &mut |c: &Case| {
        s.cases += 1;
        if c.exit == Exit::Slow {
            s.bails += 1;
            s.already_latched += usize::from(c.after.sys.stky == c.before.stky);
        }
    });
    s
}

#[test]
fn the_bail_share_on_the_census_programs_is_measured_and_small() {
    let pool = census_encodings(|k, _| matches!(k, Kind::T2a | Kind::T2b | Kind::T6n));
    let programs: Vec<Vec<u16>> = pool.iter().map(|p| p.to_vec()).collect();
    let s = measure(&programs);
    eprintln!("fixed uop bail share on the census sweep: {s:?}");
    assert!(s.cases >= 400, "{s:?}");
    assert!(s.bails > 0, "the sweep reaches the exception cases: {s:?}");
    // MEASURED (this test, 2026-11-28): 3 of 480 cases bail (0.6%), 1 of them on bits STKY
    // already held. The documented numbers of the module doc of `h_fixed`.
    assert!(
        s.bails * 50 < s.cases && s.already_latched <= s.bails,
        "more than 2% of the cases bail: {s:?}"
    );
}

#[test]
fn the_bail_share_on_edge_biased_random_alu_programs_is_the_upper_bound() {
    // the states of the rig are random words biased to the edges (0, 1, the sign bit, ...) and
    // STKY is random, so an overflow is far more frequent than in code that runs on audio data
    let programs = random_programs(0, &ALU_OPS, 10, &mut Rng::new(0xA1));
    let s = measure(&programs);
    eprintln!("fixed uop bail share on random ALU programs: {s:?}");
    assert!(s.cases >= 800 && s.bails > 0, "{s:?}");
    // MEASURED (this test, 2026-11-28): 24 of 840 cases bail (2.9%), 13 of them on bits STKY
    // already held; a bound for code on edge values, not a result for firmware
    assert!(s.bails * 4 < s.cases, "more than 25% bail: {s:?}");
    assert!(
        s.already_latched > 0,
        "some bails raised only latched bits: {s:?}"
    );
}

#[test]
fn an_already_latched_sticky_bit_still_pokes_the_controller_so_it_is_not_a_no_op() {
    // R3 = R1 + R2 overflows with AOS already set in STKY: the interpreter still raises FIXI
    let tables = Tables::new();
    let latched = |c: &mut Core| {
        (c.r[1], c.r[2]) = (0x7FFF_FFFF, 1);
        c.sys.stky[0] |= STKY_AOS;
    };
    let (case, _) = tables
        .run(&t2a(31, 0x1312), 1, false, &latched)
        .unwrap()
        .expect("the family lowers the add");
    assert_eq!(case.exit, Exit::Slow, "it bails although AOS is latched");
    assert!(case.raised);
    assert_eq!(case.after.sys.stky[0] & STKY_AOS, STKY_AOS);
    assert_eq!(
        case.after.irq.due, 0,
        "the raise poked the controller again"
    );
    // and what the case did not need: a clear STKY bit changes STKY, a latched one does not
    let clear = |c: &mut Core| {
        (c.r[1], c.r[2]) = (0x7FFF_FFFF, 1);
        c.sys.stky[0] &= !STKY_AOS;
    };
    let (fresh, _) = tables
        .run(&t2a(31, 0x1312), 1, false, &clear)
        .unwrap()
        .unwrap();
    assert_eq!(fresh.exit, Exit::Slow);
    assert_ne!(fresh.before.stky, fresh.after.sys.stky);
    assert_eq!(fresh.after.irq.due, 0);
}
