//! OWNER: W2-NDP (tests). The differential suite of the native data-processing lowering: for
//! every kind, the native function and the tier-2 executor run from the same state and must
//! agree on registers, all flags, the instruction count and the exit (`harness::Rig::check`).
//!
//! Every test asserts that its sweep (`sweeps.rs`) exercised something: every id wrote a
//! register or flags in some case, every flag bit changed, was set and was clear, every
//! condition held and failed, so a comparison of two runs that did nothing cannot pass.

use super::harness::{lower_test, Rig, PC};
use super::sweeps::*;
use crate::cpu::Cpu;
use crate::machine::sb::native::codegen::build_function_with;
use crate::machine::sb::testkit::{fuzz_cases, Rng};
use crate::machine::sb::uop::{k, Uop};
use cranelift_codegen::isa::CallConv;

fn cases(n: u64) -> u32 {
    fuzz_cases(n) as u32
}

#[test]
fn every_generic_id_equals_tier_2_and_each_exercises_its_effect() {
    let Some(mut rig) = Rig::new() else { return };
    let mut rng = Rng::new(0x4E44_5001);
    let all = sweep_generic(&mut rig, &mut rng, 0..96, 4, cases(40)).unwrap();
    assert_eq!(all.len(), 96);
    for (kind, t) in &all {
        assert_eq!(t.cases, 4 * u64::from(cases(40)), "kind {kind}");
        let (op, s) = (kind >> 1 & 15, kind & 1 != 0);
        if !matches!(op, 8..=11) {
            assert!(t.reg_changed > 0, "kind {kind} never changed a register");
        }
        if s {
            assert!(t.flags_changed > 0, "kind {kind} never changed a flag");
        } else {
            assert_eq!(t.flags_changed, 0, "kind {kind}: no S, no flag");
        }
    }
}

#[test]
fn the_specialised_ids_equal_tier_2_and_each_exercises_its_effect() {
    let Some(mut rig) = Rig::new() else { return };
    let mut rng = Rng::new(0x4E44_5002);
    let all = sweep_specials(&mut rig, &mut rng, 4, cases(48)).unwrap();
    assert_eq!(all.len(), 12);
    for (kind, t) in &all {
        let flags = matches!(*kind, k::CMP_IMM | k::CMP_REG | k::TST_IMM | k::TST_REG);
        assert_eq!(t.cases, 4 * u64::from(cases(48)));
        assert_eq!(t.flags_changed > 0, flags, "kind {kind} {t:?}");
        assert_eq!(t.reg_changed > 0, !flags, "kind {kind} {t:?}");
    }
}

#[test]
fn arithmetic_on_edge_operand_pairs_and_every_flag_nibble_equals_tier_2() {
    let Some(mut rig) = Rig::new() else { return };
    let mut rng = Rng::new(0x4E44_5003);
    let t = sweep_arith_edges(&mut rig, &mut rng, &[2, 3, 4, 5, 6, 7, 10, 11], &VALS).unwrap();
    assert_eq!(t.cases, 8 * 2 * 4 * 13 * 13 * 16);
    assert!(t.all_flag_bits_exercised(), "{t:?}");
}

#[test]
fn every_static_shift_type_and_amount_gives_the_interpreters_value_and_carry() {
    let Some(mut rig) = Rig::new() else { return };
    let mut rng = Rng::new(0x4E44_5004);
    let amounts: Vec<u32> = (0..32).collect();
    let t = sweep_shift_imm(&mut rig, &mut rng, &[0, 1, 2, 3], &amounts).unwrap();
    assert_eq!(t.cases, 4 * 32 * 2 * 13 * 16);
    assert!(
        t.flag_bits_exercised(0b1110),
        "N Z C move, V never does: {t:?}"
    );
}

#[test]
fn register_shift_amounts_0_to_300_and_noise_in_the_high_bits_equal_tier_2() {
    let Some(mut rig) = Rig::new() else { return };
    let mut rng = Rng::new(0x4E44_5005);
    let amounts: Vec<u32> = (0..=300).collect();
    let t = sweep_shift_reg(&mut rig, &mut rng, &[0, 1, 2, 3], &amounts).unwrap();
    assert_eq!(t.cases, 4 * 2 * 301 * 13 * 16);
    assert!(
        t.flag_bits_exercised(0b1110),
        "N Z C move, V never does: {t:?}"
    );
    // the boundary amounts of the rules, with the high bits as noise
    let t = sweep_shift_reg(&mut rig, &mut rng, &[0, 1, 2, 3], &AMOUNTS).unwrap();
    assert!(t.cases > 0 && t.flag_bits_exercised(0b1110));
}

#[test]
fn rotated_immediates_with_and_without_rotation_equal_tier_2() {
    let Some(mut rig) = Rig::new() else { return };
    let rots: Vec<u32> = (0..16).collect();
    let t = sweep_immediates(&mut rig, &mut Rng::new(0x4E44_5006), &rots, &[0, 13, 5, 10]).unwrap();
    assert_eq!(t.cases, 16 * 4 * 4 * 8);
    assert!(t.all_flag_bits_exercised(), "{t:?}");
}

#[test]
fn conditional_uops_run_their_body_exactly_when_tier_2_does_for_every_condition() {
    let Some(mut rig) = Rig::new() else { return };
    let conds: Vec<u8> = (0..16).collect();
    let t = sweep_pred(&mut rig, &mut Rng::new(0x4E44_5007), &conds).unwrap();
    assert_eq!(t.cases, 16 * 16 * 6);
    assert!(t.all_flag_bits_exercised(), "{t:?}");
}

#[test]
fn a_flag_overwritten_before_any_use_costs_no_code() {
    let Some(mut rig) = Rig::new() else { return };
    let cmp = |ra, rb| {
        let mut u = Uop::new(k::CMP_REG, PC);
        (u.rn, u.rm) = (ra, rb);
        u
    };
    let bytes = |rig: &mut Rig, uops: &[Uop]| {
        let before = rig.code_bytes();
        rig.compile(&Rig::trace(uops));
        rig.code_bytes() - before
    };
    let one = bytes(&mut rig, &[cmp(1, 2)]);
    // the first compare's flags are overwritten by the second before any exit or read
    let twice = bytes(&mut rig, &[cmp(3, 4), cmp(1, 2)]);
    assert!(one > 0);
    assert_eq!(twice, one, "the dead compare left no code");
    // a compare whose flags are read by the next uop is not dead
    let mut adc = Uop::new(k::dp(1, 5, false), PC);
    (adc.rd, adc.rn, adc.rm) = (5, 3, 4);
    let used = bytes(&mut rig, &[cmp(3, 4), adc]);
    assert!(used > one, "a live compare stays");
}

#[test]
fn a_kind_it_does_not_lower_leaves_natively_before_it_with_the_state_exact() {
    // IR level (everywhere): the walk ends the trace with a generic exit at the unknown uop
    let mut mov = Uop::new(k::MOV_IMM, PC);
    (mov.rd, mov.imm) = (1, 7);
    for kind in [108u8, 110, 111] {
        let sb = Rig::trace(&[mov, Uop::new(kind, PC + 4)]);
        let func = build_function_with(
            &sb,
            CallConv::SystemV,
            cranelift_codegen::ir::types::I64,
            lower_test,
        )
        .expect("the unknown uop becomes a generic exit");
        let text = func.display().to_string();
        assert_eq!(text.matches("return").count(), 1, "kind {kind}: {text}");
        assert!(text.contains("4100"), "r15 = 0x1004: {text}");
    }
    let Some(mut rig) = Rig::new() else { return };
    let sb = Rig::trace(&[mov, Uop::new(108, PC + 4), mov]);
    let native = rig.compile(&sb);
    let (out, cpu) = rig.run_raw(&native, &Cpu::new());
    assert!(out.is_generic());
    assert_eq!(
        (out.target, cpu.r[15], cpu.r[1], cpu.insns),
        (PC + 4, PC + 4, 7, 1)
    );
}
