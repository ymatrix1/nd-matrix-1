//! OWNER: W2-NMEM. Tests of the native memory lowerer: decisions and IR everywhere, execution
//! against tier 2 where native code is available (otherwise each test prints why it skipped and
//! asserts the fallback), and the mutation check of the memory and flow mutants.

use super::harness::compare::{run_case, same_state, Case};
use super::harness::oracle::{Slow, Tally};
use super::harness::*;
use super::lower_mem;
use super::suites::{block_suite, shift_and_abs_suite};
use crate::machine::sb::exec_mem::{exec_mem, F_P, F_U, F_WB};
use crate::machine::sb::native::codegen::build_function_with;
use crate::machine::sb::native::emit::testing::{build_and_verify, verified};
use crate::machine::sb::native::emit::Unsupported;
use crate::machine::sb::native::rt::Jit;
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::uop::{k, Exit, ExitKind, Sb, Uop};
use crate::machine::SysBus;
use cranelift_codegen::isa::CallConv;

/// `u` followed by `EXIT`; exit 0 is the stale exit of memory uops, exit 1 the `EXIT`.
pub(crate) fn one(u: Uop) -> Sb {
    let exits = vec![
        Exit::new(ExitKind::Stale, 1, u.pc, u.pc + 4),
        Exit::new(ExitKind::Direct, 1, u.pc, u.pc + 4),
    ];
    let uops = vec![u, Uop::exit(1, u.pc)];
    Sb::from_parts(0x1000, 2, uops, exits, vec![(0x1000, 0)]).unwrap()
}

/// `traces` random traces of each kind in `kinds`, `cases` random states each, all compared.
pub(super) fn sweep(
    jit: &mut Jit,
    rng: &mut Rng,
    kinds: &[u8],
    traces: u32,
    cases: u32,
) -> Result<Tally, String> {
    let mut t = Tally::default();
    for &kind in kinds {
        for n in 0..traces {
            let structured = n % 3 != 2;
            let u = random_mem_uop_of(rng, kind, 0x1000, structured);
            let sb = one(u);
            let native = compile_test(jit, &sb).ok_or("the trace did not compile")?;
            for c in 0..cases {
                let case = Case::new(random_state(rng, structured), c % 3);
                run_case(jit, &native, &sb, &case, &mut t)
                    .map_err(|e| format!("kind {kind} uop {u:?}: {e}"))?;
            }
        }
    }
    Ok(t)
}

/// The mixed trace suite: random traces with guards and predicated bodies.
pub(super) fn trace_suite(
    jit: &mut Jit,
    rng: &mut Rng,
    traces: u32,
    cases: u32,
) -> Result<Tally, String> {
    let mut t = Tally::default();
    for n in 0..traces {
        let structured = n % 3 != 2;
        let b = random_trace(rng, structured);
        let native = compile_test(jit, &b.sb).ok_or("the trace did not compile")?;
        for c in 0..cases {
            let mut cpu = random_state(rng, structured);
            for (r, v) in &b.eq_guards {
                if rng.flag() {
                    cpu.r[*r as usize] = *v;
                }
            }
            run_case(jit, &native, &b.sb, &Case::new(cpu, c % 3), &mut t)
                .map_err(|e| format!("trace {n}: {e}"))?;
        }
    }
    Ok(t)
}

#[test]
fn every_generated_trace_lowers_to_verified_ir_and_a_whole_function() {
    let mut rng = Rng::new(0x4E4D);
    let mut kinds_seen = [false; 256];
    for n in 0..120 {
        let b = random_trace(&mut rng, n % 3 != 2);
        let func = build_function_with(
            &b.sb,
            CallConv::SystemV,
            cranelift_codegen::ir::types::I64,
            test_lower,
        )
        .expect("every uop of the generated traces lowers and the trace ends in an exit");
        verified(func);
        for u in b.sb.uops.iter() {
            kinds_seen[u.kind as usize] = true;
        }
    }
    for kind in MEM_KINDS {
        assert!(kinds_seen[kind as usize], "kind {kind} was never generated");
    }
}

#[test]
fn unsupported_shapes_are_declined_before_anything_is_emitted() {
    let sb = one(Uop::new(k::LDR_IMM, 0x1000));
    let uop = |kind: u8, edit: &dyn Fn(&mut Uop)| {
        let mut u = Uop::new(kind, 0x1000);
        u.sh = 1;
        edit(&mut u);
        u
    };
    let bad = [
        uop(k::LDR_IMM, &|u| u.rn = 15),
        uop(k::LDR_IMM, &|u| u.rd = 15),
        uop(k::STR_IMM, &|u| u.rn = 15),
        uop(k::LDR_REG, &|u| (u.rm, u.sh) = (15, 1)),
        uop(k::STRB_REG, &|u| u.sh = 3 << 5), // ROR #0 is RRX
        uop(k::LDRH, &|u| (u.rs, u.rm) = (8, 15)),
        uop(k::LDRD, &|u| u.rd = 14),
        uop(k::STRD, &|u| u.rd = 15),
        uop(k::LDM, &|u| (u.imm, u.sh) = (0, 0)),
        uop(k::STM, &|u| (u.imm, u.sh) = (0x8001, 2)),
        uop(k::LDM, &|u| (u.rn, u.imm, u.sh) = (15, 1, 1)),
        uop(k::LDR_ABS, &|u| u.rd = 15),
        Uop::new(129, 0x1000),
        Uop::new(k::ADD_IMM, 0x1000),
    ];
    build_and_verify(&sb, |cx| {
        let before = cx.b.func.dfg.num_insts();
        for u in &bad {
            assert_eq!(lower_mem(cx, u), Err(Unsupported), "{u:?}");
        }
        assert_eq!(cx.b.func.dfg.num_insts(), before, "nothing was emitted");
        assert_eq!((cx.dirty, cx.flags_def), (0, 0));
        cx.exit_generic(0x1000);
    });
}

#[test]
fn a_word_load_checks_the_entry_rotates_and_a_store_checks_the_code_flag_with_the_store_mask() {
    let ir = |u: Uop| {
        build_and_verify(&one(u), |cx| {
            lower_mem(cx, &u).unwrap();
            cx.exit_generic(0x1000);
        })
        .display()
        .to_string()
    };
    let mut ldr = Uop::new(k::LDR_IMM, 0x1000);
    (ldr.rd, ldr.rn, ldr.rs) = (1, 0, F_U | F_P);
    let text = ir(ldr);
    assert!(text.contains("rotr"), "unaligned rotate: {text}");
    assert!(text.contains("brif"), "the side exit: {text}");
    assert!(
        text.contains(&format!("{}", 0x8FFF_FFFFu32 as i32)),
        "load mask: {text}"
    );
    let mut str_ = Uop::new(k::STR_IMM, 0x1000);
    (str_.rd, str_.rn, str_.rs) = (1, 0, F_U | F_P);
    let text = ir(str_);
    assert!(text.contains("0x5fff_ffff"), "store mask: {text}");
    assert!(!text.contains("rotr"), "{text}");
    let mut ldm = Uop::new(k::LDM, 0x1000);
    (ldm.rn, ldm.imm, ldm.sh, ldm.rs) = (0, 0b1110, 3, F_U | F_WB);
    let text = ir(ldm);
    assert_eq!(
        text.matches("brif").count(),
        1,
        "one entry check for the whole block: {text}"
    );
}

#[test]
fn every_kind_equals_tier_2_on_random_states_and_the_slow_cases_leave_before_any_effect() {
    let Some(mut jit) = native_or_skip("every_kind_equals_tier_2") else {
        return;
    };
    let mut rng = Rng::new(0xA11CE);
    for kind in MEM_KINDS {
        let t = sweep(&mut jit, &mut rng, &[kind], 8, 14).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            t.completed >= 20,
            "kind {kind} completed only {} cases: {t:?}",
            t.completed
        );
        assert!(
            t.loads + t.stores > 0,
            "kind {kind} never reached memory: {t:?}"
        );
        assert!(
            t.slow_exits >= 2,
            "kind {kind} never left for the interpreter: {t:?}"
        );
    }
}

#[test]
fn random_traces_with_guards_and_predicated_bodies_equal_tier_2() {
    let Some(mut jit) = native_or_skip("random_traces_equal_tier_2") else {
        return;
    };
    let mut rng = Rng::new(0x7ACE);
    let t = trace_suite(&mut jit, &mut rng, 120, 10).unwrap_or_else(|e| panic!("{e}"));
    assert!(t.completed > 400 && t.slow_exits > 150, "{t:?}");
    for why in [
        Slow::Unmapped,
        Slow::HookRead,
        Slow::HookWrite,
        Slow::Code,
        Slow::PageCross,
    ] {
        assert!(
            t.slow_of(why) >= 5,
            "{why:?} seen {} times: {t:?}",
            t.slow_of(why)
        );
    }
    assert!(t.loads > 200 && t.stores > 200, "{t:?}");
    assert!(t.unaligned_word_loads > 20 && t.odd_halfwords > 10, "{t:?}");
    assert!(t.blocks > 20 && t.pairs > 5 && t.writebacks > 100, "{t:?}");
    assert!(t.guards_taken > 20 && t.guards_passed > 20, "{t:?}");
    assert!(t.preds_run > 20 && t.preds_skipped > 20, "{t:?}");
}

#[test]
fn ldm_and_stm_equal_tier_2_for_every_addressing_mode_and_base_position() {
    let Some(mut jit) = native_or_skip("ldm_and_stm_equal_tier_2") else {
        return;
    };
    let t = block_suite(&mut jit, &mut Rng::new(0xB10C4), 16).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        t.blocks > 200 && t.slow_of(Slow::PageCross) > 20 && t.writebacks > 100,
        "{t:?}"
    );
}

#[test]
fn register_offsets_of_every_shift_type_and_the_absolute_literal_load_equal_tier_2() {
    let Some(mut jit) = native_or_skip("register_offsets_and_ldr_abs") else {
        return;
    };
    let t = shift_and_abs_suite(&mut jit, &mut Rng::new(0x5417)).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        t.completed > 200 && t.unaligned_word_loads > 4 && t.slow_exits >= 4,
        "{t:?}"
    );
}

/// Word loads at every alignment, with the base equal to the destination, and halfword and byte
/// accesses at odd addresses and at the last byte of a page.
#[test]
fn unaligned_and_page_end_accesses_equal_tier_2() {
    let Some(mut jit) = native_or_skip("unaligned_and_page_end_accesses") else {
        return;
    };
    let mut t = Tally::default();
    let mut rng = Rng::new(0xFA11);
    for kind in [
        k::LDR_IMM,
        k::LDRB_IMM,
        k::LDRH,
        k::LDRSH,
        k::LDRSB,
        k::STR_IMM,
        k::STRB_IMM,
        k::STRH,
    ] {
        for (rd, rn) in [(1u8, 0u8), (0, 0)] {
            for flags in [F_U | F_P, F_P, F_U | F_P | F_WB, F_WB | F_U, F_WB] {
                let mut u = Uop::new(kind, 0x1000);
                (u.rd, u.rn, u.rs, u.imm) = (rd, rn, flags, 0);
                let sb = one(u);
                let native = compile_test(&mut jit, &sb).unwrap();
                for (n, off) in [0u32, 1, 2, 3, 0xFFC, 0xFFD, 0xFFE, 0xFFF]
                    .into_iter()
                    .enumerate()
                {
                    let mut cpu = random_state(&mut rng, true);
                    cpu.r[0] = page_addr(RAM0 + (n as u32 & 1)) + off;
                    run_case(&jit, &native, &sb, &Case::new(cpu, n as u32), &mut t).unwrap_or_else(
                        |e| panic!("kind {kind} rd {rd} flags {flags} off {off:#x}: {e}"),
                    );
                }
            }
        }
    }
    assert!(
        t.completed > 500 && t.unaligned_word_loads > 20 && t.odd_halfwords > 20,
        "{t:?}"
    );
}

/// I6: a store that makes decoded code stale is a slow case of the fast path (the page carries
/// `HOOK_C`); the interpreter performs it and the driver finds `stale_code` non-empty. Native
/// code up to the store plus that step is the state tier 2 reaches through its `Stale` exit.
#[test]
fn a_store_to_a_code_page_leaves_before_the_store_and_the_step_equals_the_tier_2_stale_exit() {
    let Some(mut jit) = native_or_skip("a_store_to_a_code_page") else {
        return;
    };
    let mut rng = Rng::new(0x57A1E);
    let mut checked = 0;
    for kind in [
        k::STR_IMM,
        k::STRB_IMM,
        k::STRH,
        k::STRD,
        k::STM,
        k::STR_REG,
        k::STRB_REG,
    ] {
        for lead in [0usize, 2] {
            // `lead` fast stores to RAM, then the store to the code page, then an ADD that must not run
            let mut uops = Vec::new();
            for j in 0..lead {
                let mut s = Uop::new(k::STR_IMM, 0x1000 + 4 * j as u32);
                (s.rd, s.rn, s.rs, s.imm) = (j as u8 + 3, 7, F_U | F_P, 4 * j as u32);
                uops.push(s);
            }
            let mut u = random_mem_uop_of(&mut rng, kind, 0x1000 + 4 * lead as u32, true);
            (u.rn, u.rm, u.rs, u.imm) = match kind {
                k::STM => (6, 0, F_U | F_P | F_WB, 0b0011_1000),
                k::STR_REG | k::STRB_REG => (6, 8, F_U | F_P, 0),
                _ => (6, 8, F_U | F_P | F_WB, 4),
            };
            u.sh = if kind == k::STM {
                3
            } else if matches!(kind, k::STR_REG | k::STRB_REG) {
                0
            } else {
                u.sh
            };
            u.rd = 4;
            u.set_exit_idx(0);
            let store_pc = u.pc;
            uops.push(u);
            uops.push(add_imm(2, 2, 1, store_pc + 4));
            uops.push(Uop::exit(1, store_pc + 4));
            let n = uops.len() as u16;
            let exits = vec![
                Exit::new(ExitKind::Stale, lead as u16 + 1, store_pc, store_pc + 4),
                Exit::new(ExitKind::Direct, n - 1, store_pc + 4, store_pc + 8),
            ];
            let sb = Sb::from_parts(0x1000, n, uops, exits, vec![(0x1000, 0)]).unwrap();
            let native = compile_test(&mut jit, &sb).unwrap();
            for c in 0..12u32 {
                let mut cpu = random_state(&mut rng, true);
                cpu.r[7] = page_addr(RAM1) + 0x100;
                cpu.r[6] = page_addr(CODE) + 0x10 + 4 * rng.below(8);
                cpu.r[8] = rng.below(8);
                let (mut cn, mut mem_n) = (cpu.clone(), world(c));
                let out = jit.run(&native, &mut cn, &mut mem_n, cpu.insns + 100);
                assert!(
                    out.is_generic() && out.target == store_pc,
                    "{kind}: {out:?}"
                );
                assert_eq!(
                    cn.insns,
                    cpu.insns + lead as u64,
                    "the stores before it retired"
                );
                assert!(mem_n.stale_code.is_empty(), "nothing was stored yet");
                // the driver: one interpreter step through SysBus, then it finds the queue non-empty
                let mut bus = SysBus::new();
                bus.mem = mem_n;
                exec_mem(&mut cn.r, &mut bus, &sb.uops[lead]);
                cn.insns += 1;
                cn.r[15] = store_pc + 4;
                assert_eq!(bus.mem.stale_code, vec![(page_addr(CODE) >> 12)], "{kind}");
                // tier 2 through its Stale exit
                let (mut ct, mut bt, mut left) = (cpu.clone(), SysBus::new(), 100u64);
                bt.mem = world(c);
                let t2 = crate::machine::sb::exec::run_sb(&mut ct, &mut bt, &sb, &mut left, false);
                assert_eq!(
                    (t2.exit, t2.target),
                    (0, store_pc + 4),
                    "{kind}: the stale exit"
                );
                same_state("stale", (&cn, &mut bus.mem), (&ct, &mut bt.mem))
                    .unwrap_or_else(|e| panic!("{kind}: {e}"));
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 7 * 2 * 12);
}
