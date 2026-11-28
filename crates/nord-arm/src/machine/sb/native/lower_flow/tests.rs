//! OWNER: W2-NMEM. Tests of the native control-flow lowerer: decisions and IR everywhere,
//! execution against tier 2 where native code is available (a test prints why it skipped and
//! asserts the fallback otherwise).

use super::cond::{cond_value, is_cond};
use super::lower_flow;
use super::suite::{cond_suite, loop_case, loop_suite, loop_trace, pair_suite};
use crate::machine::sb::native::emit::testing::build_and_verify;
use crate::machine::sb::native::emit::Unsupported;
use crate::machine::sb::native::lower_mem::harness::oracle::{run_case, Case, Tally};
use crate::machine::sb::native::lower_mem::harness::{
    add_imm, compile_test, native_or_skip, random_state, test_lower,
};
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::uop::fused::fuse_pair;
use crate::machine::sb::uop::{k, Exit, ExitKind, Sb, Uop};

/// A trace of `uops` with the `exits` and an `n_guest` of 4.
fn trace(uops: Vec<Uop>, exits: Vec<Exit>) -> Sb {
    Sb::from_parts(0x1000, 4, uops, exits, vec![(0x1000, 0)]).unwrap()
}

fn direct(done: u16) -> Exit {
    Exit::new(ExitKind::Direct, done, 0x1000, 0x2000 + u32::from(done))
}

#[test]
fn a_condition_is_a_value_for_every_code_below_16_and_nothing_above() {
    let sb = trace(vec![Uop::exit(0, 0x1000)], vec![direct(1)]);
    build_and_verify(&sb, |cx| {
        let before = cx.b.func.dfg.num_insts();
        for cond in [16u8, 17, 200, 255] {
            assert!(!is_cond(cond) && cond_value(cx, cond).is_none(), "{cond}");
        }
        assert_eq!(cx.b.func.dfg.num_insts(), before, "nothing emitted");
        for cond in 0..16u8 {
            assert!(is_cond(cond) && cond_value(cx, cond).is_some(), "{cond}");
        }
        cx.exit_generic(0x1000);
    });
}

#[test]
fn control_shapes_native_code_cannot_take_are_declined_before_anything_is_emitted() {
    let mut dynamic = direct(1);
    dynamic.kind = ExitKind::Dynamic;
    let mut via_r15 = direct(1);
    (via_r15.kind, via_r15.reg) = (ExitKind::Indirect, 15);
    let mut looped = direct(1);
    looped.kind = ExitKind::LoopBack;
    let sb = trace(
        vec![Uop::exit(0, 0x1000)],
        vec![direct(1), dynamic, via_r15, looped],
    );
    let with = |kind: u8, edit: &dyn Fn(&mut Uop)| {
        let mut u = Uop::new(kind, 0x1000);
        edit(&mut u);
        u
    };
    let bad = [
        with(k::GENERIC, &|u| u.imm = 0xE1A0_0000),
        with(k::GENERIC_END, &|u| u.rs = 0),
        with(k::MRS, &|_| ()),
        with(k::MSR_C, &|_| ()),
        with(k::GUARD, &|u| u.cond = 16),
        with(k::GUARD, &|u| u.rs = 1),
        with(k::GUARD, &|u| u.rs = 3),
        with(k::GUARD, &|u| u.rs = 9),
        with(k::GUARD_REG_EQ, &|u| u.rn = 15),
        with(k::GUARD_REG_EQ, &|u| u.rs = 2),
        with(k::EXIT, &|u| u.rs = 1),
        with(k::EXIT_REG, &|u| u.rs = 2),
        with(k::EXIT_REG_ARM, &|u| u.rs = 3),
        with(k::PRED, &|u| u.cond = 16),
        with(k::LOOP_BACK, &|u| u.rs = 0), // the trace has no loop header
        with(k::LOOP_BACK, &|u| u.rs = 3),
        with(k::TST_GUARD_I, &|u| u.rs = 0), // the stand-in lowerer has no TST
        with(k::ADD_IMM, &|_| ()),
    ];
    build_and_verify(&sb, |cx| {
        cx.set_lowerer(test_lower);
        let before = cx.b.func.dfg.num_insts();
        for u in &bad {
            assert_eq!(lower_flow(cx, u), Err(Unsupported), "{u:?}");
        }
        assert_eq!(cx.b.func.dfg.num_insts(), before, "nothing was emitted");
        cx.exit_generic(0x1000);
    });
}

#[test]
fn a_pred_whose_body_is_not_native_hands_over_at_the_pred_with_the_first_instruction_exact() {
    let Some(mut jit) = native_or_skip("a_pred_whose_body_is_not_native") else {
        return;
    };
    let mut pred = Uop::new(k::PRED, 0x1004);
    (pred.cond, pred.imm, pred.n) = (14, 2, 0);
    let uops = vec![
        add_imm(1, 1, 5, 0x1000),
        pred,
        add_imm(2, 2, 7, 0x1004),
        Uop::generic(0xE1A0_0000, 0x1008, 0),
        Uop::exit(1, 0x1008),
    ];
    let sb = trace(uops, vec![direct(3), direct(4)]);
    let native = compile_test(&mut jit, &sb).expect("the trace ends in a generic exit");
    let mut cpu = random_state(&mut Rng::new(1), true);
    let before = cpu.clone();
    let out = jit.run(&native, &mut cpu, &mut crate::mem::Mem::new(), 100);
    assert!(out.is_generic(), "{out:?}");
    assert_eq!((out.target, cpu.r[15]), (0x1004, 0x1004), "at the PRED");
    assert_eq!(
        cpu.r[1],
        before.r[1].wrapping_add(5),
        "the add before it ran"
    );
    assert_eq!(
        (cpu.r[2], cpu.insns),
        (before.r[2], before.insns + 1),
        "the body did not"
    );
}

#[test]
fn every_condition_code_equals_tier_2_on_all_sixteen_flag_states() {
    let Some(mut jit) = native_or_skip("every_condition_code_equals_tier_2") else {
        return;
    };
    let t = cond_suite(&mut jit, &mut Rng::new(0xC0DE)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(t.cases, 16 * 16);
    assert!(
        t.guards_taken >= 8 * 16 && t.guards_passed >= 2 * 16,
        "{t:?}"
    );
    assert_eq!(
        t.guards_taken + t.guards_passed,
        16 * 16,
        "both outcomes of every condition: {t:?}"
    );
}

#[test]
fn loops_equal_tier_2_for_every_budget_the_watch_exit_and_fused_and_unfused_guards() {
    let Some(mut jit) = native_or_skip("loops_equal_tier_2") else {
        return;
    };
    let t = loop_suite(&mut jit, &mut Rng::new(0x100B), 40, 14).unwrap_or_else(|e| panic!("{e}"));
    assert!(t.back_edges > 150, "the loops looped: {t:?}");
    assert!(t.completed > 200 && t.slow_exits > 30, "{t:?}");
    assert!(
        t.exits_by_index[0] > 40,
        "budget exits through the LoopBack exit: {t:?}"
    );
    assert!(t.exits_by_index[1] > 40, "guard exits: {t:?}");
}

#[test]
fn the_back_edge_counts_the_budget_exactly_at_every_boundary_and_the_watch_exits_at_once() {
    let Some(mut jit) = native_or_skip("the_back_edge_counts_the_budget") else {
        return;
    };
    let mut rng = Rng::new(0xB0DE);
    let mut lt = loop_trace(&mut rng, false);
    while lt.limit < 4 {
        lt = loop_trace(&mut rng, false);
    }
    let native = compile_test(&mut jit, &lt.sb).unwrap();
    let n = u64::from(lt.sb.n_guest);
    let mut t = Tally::default();
    for left in n..=6 * n {
        for watch in [false, true] {
            let mut case = loop_case(&mut rng, &lt, watch);
            case.cpu.r[5] = 0;
            for (r, reg) in case.cpu.r.iter_mut().enumerate().take(4) {
                *reg = crate::machine::sb::native::lower_mem::harness::page_addr(
                    crate::machine::sb::native::lower_mem::harness::RAM0,
                ) + 0x100 * (r as u32 + 1);
            }
            case.left = left;
            run_case(&jit, &native, &lt.sb, &case, &mut t)
                .unwrap_or_else(|e| panic!("left {left} watch {watch}: {e}"));
        }
    }
    assert!(t.back_edges >= 20 && t.exits_by_index[0] >= 10, "{t:?}");
    assert!(
        t.exits_by_index[1] >= 3,
        "the guard ended a loop too: {t:?}"
    );
}

#[test]
fn every_exit_form_equals_tier_2_including_interworking_and_the_stale_and_watch_kinds() {
    let Some(mut jit) = native_or_skip("every_exit_form_equals_tier_2") else {
        return;
    };
    let mut rng = Rng::new(0xE817);
    let mut t = Tally::default();
    let forms = [
        (ExitKind::Direct, k::EXIT),
        (ExitKind::Fault, k::EXIT),
        (ExitKind::Stale, k::EXIT),
        (ExitKind::Watch, k::EXIT),
        (ExitKind::Indirect, k::EXIT_REG),
        (ExitKind::IndirectArm, k::EXIT_REG_ARM),
    ];
    for (kind, uop_kind) in forms {
        for guarded in [false, true] {
            for reg in [0u8, 7, 14] {
                let mut e = Exit::new(kind, 3, 0x1008, 0x2468);
                e.reg = reg;
                let mut exit = Uop::exit(1, 0x1008);
                exit.kind = uop_kind;
                let mut g = Uop::new(k::GUARD, 0x1004);
                (g.cond, g.rs, g.n) = (rng.below(14) as u8, 0, 0);
                let uops = if guarded { vec![g, exit] } else { vec![exit] };
                let sb = trace(uops, vec![e.clone(), e]);
                let native = compile_test(&mut jit, &sb).unwrap();
                for c in 0..24 {
                    let mut cpu = random_state(&mut rng, c % 2 == 0);
                    cpu.thumb = rng.flag();
                    cpu.r[reg as usize] = rng.next();
                    run_case(&jit, &native, &sb, &Case::new(cpu, 0), &mut t)
                        .unwrap_or_else(|e| panic!("{kind:?} guarded {guarded} r{reg}: {e}"));
                }
            }
        }
    }
    assert_eq!(t.cases, 6 * 2 * 3 * 24);
    assert!(
        t.exits_by_index[0] > 100 && t.exits_by_index[1] > 100,
        "{t:?}"
    );
}

#[test]
fn fused_uops_equal_tier_2_and_their_unfused_expansion() {
    let Some(mut jit) = native_or_skip("fused_uops_equal_tier_2") else {
        return;
    };
    let t = pair_suite(&mut jit, &mut Rng::new(0xFA57), 24, 12).unwrap_or_else(|e| panic!("{e}"));
    assert!(t.completed > 100 && t.slow_exits > 20, "{t:?}");
    assert!(
        t.slow_of(crate::machine::sb::native::lower_mem::harness::oracle::Slow::HookRead) > 0,
        "{t:?}"
    );
    assert!(t.guards_taken > 20 && t.guards_passed > 20, "{t:?}");
}

#[test]
fn a_fused_uop_whose_halves_do_not_both_lower_is_declined_with_nothing_emitted() {
    let mut tst = Uop::new(k::TST_IMM, 0x1000);
    (tst.rn, tst.imm) = (1, 0xFF);
    let mut g = Uop::new(k::GUARD, 0x1004);
    (g.cond, g.rs) = (0, 0);
    let fused = fuse_pair(&tst, &g).expect("tst + guard");
    assert_eq!(fused.kind, k::TST_GUARD_I);
    let sb = trace(vec![fused, Uop::exit(0, 0x1004)], vec![direct(2)]);
    build_and_verify(&sb, |cx| {
        cx.set_lowerer(test_lower);
        let before = cx.b.func.dfg.num_insts();
        assert_eq!(lower_flow(cx, &fused), Err(Unsupported));
        assert_eq!(cx.b.func.dfg.num_insts(), before);
        assert_eq!(cx.retired, 0);
        cx.exit_generic(0x1000);
    });
}
