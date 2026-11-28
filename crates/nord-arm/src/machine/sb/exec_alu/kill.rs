//! The specialised ids against the interpreter and the generic id they replace, the comparator
//! self-check, and the mutation kill test (WP W1-XALU). A comparator that cannot fail, or a
//! mutant site nothing observes, would make the differential suite (`diff.rs`) vacuous.

use super::diff::{build, check, cpu_with, random_case, Tally, P};
use super::*;
use crate::machine::sb::mutants;
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::uop::pack_flags;

/// `(kind, form, opcode, s)` of the specialised ids and the generic id each replaces.
const SPECIAL: [(u8, u32, u32, bool); 12] = [
    (k::MOV_IMM, 0, 13, false),
    (k::MOV_REG, 1, 13, false),
    (k::MVN_IMM, 0, 15, false),
    (k::ADD_IMM, 0, 4, false),
    (k::SUB_IMM, 0, 2, false),
    (k::ADD_REG, 1, 4, false),
    (k::SUB_REG, 1, 2, false),
    (k::CMP_IMM, 0, 10, true),
    (k::CMP_REG, 1, 10, true),
    (k::TST_IMM, 0, 8, true),
    (k::TST_REG, 1, 8, true),
    (k::ORR_IMM, 0, 12, false),
];

/// `n` random cases per specialised id: against `dp_exec` and against the generic uop.
fn check_special(rng: &mut Rng, n: u32, t: &mut Tally) -> Result<(), String> {
    for &(kind, form, op, s) in &SPECIAL {
        for _ in 0..n {
            let (cpu, mut c) = random_case(rng, form, op, s, true);
            let generic = c.u;
            c.u.kind = kind;
            if !s {
                c.u.sh = 0; // specialised non-S uops keep sh == 0 (design note 3.1)
            }
            check(&cpu, &c, t).map_err(|e| format!("special kind {kind}: {e}"))?;
            let (mut a, mut fa) = (cpu.r, pack_flags(&cpu));
            let (mut b, mut fb) = (cpu.r, pack_flags(&cpu));
            exec_dp(&mut a, &mut fa, &c.u);
            exec_dp(&mut b, &mut fb, &generic);
            if (a, fa) != (b, fb) {
                return Err(format!(
                    "kind {kind} differs from generic id {}",
                    generic.kind
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn the_specialised_ids_equal_the_interpreter_and_the_generic_id_they_replace() {
    let n = 2000;
    let mut t = Tally::default();
    check_special(&mut Rng::new(0x5BEC), n, &mut t).unwrap();
    assert_eq!(t.cases, n * SPECIAL.len() as u32);
    assert!(
        t.reg_changed > t.cases / 4 && t.flags_changed > t.cases / 8,
        "{t:?}"
    );
    assert_eq!((t.seen_set, t.seen_clear), (0xF, 0xF));
}

#[test]
fn the_comparator_is_not_vacuous() {
    // an adds executed as a subs must be caught in nearly every case
    let mut rng = Rng::new(7);
    let (mut caught, n) = (0, 500);
    for _ in 0..n {
        let (cpu, mut c) = random_case(&mut rng, 1, 4, true, false);
        c.u.kind = k::dp(1, 2, true);
        caught += check(&cpu, &c, &mut Tally::default()).is_err() as u32;
    }
    assert!(caught > n * 9 / 10, "caught {caught} of {n}");
    // the right uop passes and counts as one case, a write to the wrong register is caught
    let cpu = cpu_with(&[(0, 77), (1, 5)], 0);
    let c = build(0, 13, true, P::default()); // movs r0, #0
    let mut t = Tally::default();
    assert!(check(&cpu, &c, &mut t).is_ok() && t.cases == 1);
    let mut stray = c;
    stray.u.rd = 14;
    assert!(check(&cpu, &stray, &mut Tally::default()).is_err());
}

/// A small run of every family, `Err` on the first difference (the oracle of the mutation test).
fn quick(seed: u64) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let mut t = Tally::default();
    for form in 0..3 {
        for op in 0..16 {
            for s in [false, true] {
                for _ in 0..120 {
                    let (cpu, c) = random_case(&mut rng, form, op, s, false);
                    check(&cpu, &c, &mut t)?;
                }
            }
        }
    }
    check_special(&mut rng, 120, &mut t)?;
    super::mul::suite(seed, 120)?;
    Ok(())
}

#[test]
fn every_planted_mutant_is_killed_and_the_unmutated_suite_passes() {
    assert_eq!(quick(1), Ok(()));
    for &(id, name) in MUTANTS {
        assert!(
            (300..350).contains(&id),
            "{name}: id {id} is outside this file's block"
        );
        assert!(
            mutants::with(id, || quick(1)).is_err(),
            "mutant {id} {name} survived"
        );
    }
    let mut ids: Vec<u32> = MUTANTS.iter().map(|m| m.0).collect();
    ids.dedup();
    assert_eq!(ids.len(), MUTANTS.len(), "ids are unique");
}
