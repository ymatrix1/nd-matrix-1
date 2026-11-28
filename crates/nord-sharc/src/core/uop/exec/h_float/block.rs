//! Block-level differential of the float uops: programs of float instructions are translated by
//! the production lowering (`translate_range`) and their uops run through the float handlers
//! under a minimal dispatch loop, against `Core::step`.
//!
//! OWNER: W2-FLOAT, test only. The loop is the dispatch protocol of P2 for the exits float
//! handlers produce (`Ok`, `Jump`) and nothing else: an `Interp` uop, which this package does not
//! own, runs as `exec_cached` of its instruction (the semantics of `INTERP`), and a `UF_FIN` uop
//! calls `finish_step` after it. It proves what `diff.rs` cannot: consecutive float uops in one
//! block (hidden extensions flowing from one to the next), the block entry guard, a float
//! instruction in a delay slot, and the hand-over to `Core::step` after an exception.
use super::support::{alu, computes, mul, prepare, t2a, SRRFH, SRRFL};
use super::*;
use crate::core::support::{core_with, CODE};
use crate::core::uop::config::UopConfig;
use crate::core::uop::exec::exit::materialize;
use crate::core::uop::hot::UopHot;
use crate::core::uop::lower::decode::decode_at;
use crate::core::uop::lower::{translate::translate_with, translate_range};
use crate::core::uop::types::{
    Block, Exit, GuardClass, GuardFail, KindFamily, Uop, UopCtx, UF_FIN, UF_LAST,
};
use crate::core::uop::verify::compare::exact_diff;
use crate::core::uop::verify::gen::Rng;
use crate::core::Core;

/// What running a block through the mini loop did.
struct Mini {
    exit: Exit,
    /// Steps the loop accounted before the exit.
    done: u32,
}

/// The dispatch loop of P2 for one block: handlers for the float uops, `exec_cached` for `Interp`.
/// Leaves the core materialized at the exit (the block end after an `Ok` run to the end).
fn run_mini(core: &mut Core, table: &[Handler], block: &Block, uops: &[Uop]) -> Mini {
    let mut ctx = UopCtx::new(core as *mut Core, core.steps, u32::MAX);
    for u in uops {
        if u.h == UopKind::INTERP.0 {
            materialize(core, &ctx, u.pc);
            let c = decode_at(core, u.pc);
            core.exec_cached(&c)
                .expect("the interpreter runs the instruction");
            ctx.done += 1;
            if core.pc != u.b {
                return Mini {
                    exit: Exit::Jump,
                    done: ctx.done,
                };
            }
            continue;
        }
        // SAFETY: `ctx.core` is `core`, not used until the handler returns; `u` is a uop of the
        // block, so for a `Jump` the next uop of the slice exists
        let r = unsafe { table[usize::from(u.h)](&mut ctx, u) };
        match Exit::from_u32(r).expect("an exit code") {
            Exit::Ok => {
                ctx.done += u32::from(u.steps);
                if u.flags & UF_FIN != 0 {
                    materialize(core, &ctx, u.pc);
                    let c = decode_at(core, u.pc);
                    let next = core.finish_step(u.pc, c.insn.kind, c.next_pc);
                    core.pc = next;
                    if next != c.next_pc {
                        return Mini {
                            exit: Exit::Jump,
                            done: ctx.done,
                        };
                    }
                }
            }
            Exit::Jump => {
                ctx.done += u32::from(u.steps);
                materialize(core, &ctx, ctx.next_pc);
                return Mini {
                    exit: Exit::Jump,
                    done: ctx.done,
                };
            }
            other => panic!("a float uop exited with {other:?}"),
        }
    }
    materialize(core, &ctx, block.end_pc);
    Mini {
        exit: Exit::Ok,
        done: ctx.done,
    }
}

/// The PEy-mode block of `core`'s program starting at `CODE` with its uops.
fn block_of(core: &mut Core, len: usize) -> (Block, Vec<Uop>) {
    block_at(core, CODE, len)
}

/// The block of `core`'s current PEy mode that starts at `pc`, among those of the `len` parcels
/// from `CODE`.
fn block_at(core: &mut Core, pc: u32, len: usize) -> (Block, Vec<Uop>) {
    let cfg = UopConfig::default();
    let (blocks, uops) = translate_range(core, &cfg, CODE, CODE + len as u32).unwrap();
    let b = blocks
        .into_iter()
        .find(|b| b.pc == pc && b.simd == core.simd())
        .expect("a block at this pc for this mode");
    let slice = uops[b.first as usize..(b.first + b.len) as usize].to_vec();
    (b, slice)
}

/// A random program of `k` float instructions (unconditional and conditional) as parcels.
fn program(rng: &mut Rng, k: usize) -> Vec<u16> {
    let mut p = Vec::new();
    for _ in 0..k {
        let c = computes(rng)[rng.below(23) as usize];
        let cond = if rng.below(3) == 0 { rng.below(32) } else { 31 };
        p.extend(t2a(cond as u16, c));
    }
    p
}

#[test]
fn a_translated_block_of_float_uops_equals_the_interpreter() {
    let table = UopHot::handler_table();
    let mut rng = Rng::new(0xB10C_0001);
    let (mut full, mut handed_over, mut uops_run) = (0, 0, 0);
    for case in 0..160u64 {
        let simd = case % 2 == 0;
        let toggle = [0, SRRFL, SRRFH, SRRFL | SRRFH][rng.below(4) as usize];
        let k = 1 + rng.below(8) as usize;
        let prog = program(&mut rng, k);
        let (mut a, mut b) = (
            prepare(&prog, case, simd, toggle),
            prepare(&prog, case, simd, toggle),
        );
        let (block, uops) = block_of(&mut b, prog.len());
        let ctx = format!("case {case} simd {simd} k {k} toggle {toggle:#x}");
        assert!(
            uops.iter()
                .all(|u| UopKind(u.h).family() == Some(KindFamily::Float)),
            "{ctx}: a float instruction stayed an Interp uop"
        );
        assert_eq!(
            (block.steps as usize, block.guard),
            (k, GuardClass::Arith),
            "{ctx}"
        );
        let ran = run_mini(&mut b, &table, &block, &uops);
        for _ in 0..ran.done {
            a.step().expect("the interpreter runs the instruction");
        }
        if let Some(d) = exact_diff(&a, &b) {
            panic!("block vs interpreter, {ctx}: {d}");
        }
        assert!(ran.done >= 1 && ran.done as usize <= k, "{ctx}");
        match ran.exit {
            Exit::Ok => full += 1,
            _ => handed_over += 1,
        }
        uops_run += ran.done;
    }
    // some blocks ran to the end, some stopped after an exception; both ways agree
    assert!(
        full > 40 && handed_over > 5,
        "{full} full, {handed_over} handed over"
    );
    assert!(uops_run > 400, "{uops_run} uops ran");
}

#[test]
fn after_an_exception_the_rest_of_the_block_runs_on_the_interpreter_and_agrees() {
    let table = UopHot::handler_table();
    // F3 = F1 + F2 (overflow), F4 = F5 * F6, F7 = F3 + F4
    let prog = [
        t2a(31, alu(0x81, 3, 1, 2)),
        t2a(31, mul(4, 5, 6)),
        t2a(31, alu(0x81, 7, 3, 4)),
    ]
    .concat();
    let max = 0x7F7F_FFFFu32;
    for simd in [false, true] {
        let (mut a, mut b) = (prepare(&prog, 5, simd, 0), prepare(&prog, 5, simd, 0));
        for c in [&mut a, &mut b] {
            (c.r[1], c.r[2], c.s[1], c.s[2]) = (max, max, max, max);
            (c.fx.tag, c.fx_y.tag) = (c.r, c.s);
            (c.sys.stky, c.irq.irptl) = ([0, 0], 0);
        }
        let (block, uops) = block_of(&mut b, prog.len());
        let ran = run_mini(&mut b, &table, &block, &uops);
        assert_eq!(
            (ran.exit, ran.done),
            (Exit::Jump, 1),
            "simd {simd}: the overflow ends the block"
        );
        assert_eq!(
            (b.pc, b.irq.due),
            (CODE + 3, 0),
            "the next instruction, controller poked"
        );
        assert_ne!(b.sys.stky[0] & 1 << 1, 0, "AVS");
        a.step().unwrap();
        assert_eq!(exact_diff(&a, &b), None, "simd {simd}: one step");
        // the hand-over: the interpreter finishes both and they still agree
        for _ in 0..2 {
            a.step().unwrap();
            b.step().unwrap();
        }
        assert_eq!(exact_diff(&a, &b), None, "simd {simd}: after the hand-over");
    }
}

#[test]
fn a_float_instruction_in_a_delay_slot_runs_with_finish_step_and_the_first_slot_is_interp() {
    let table = UopHot::handler_table();
    // JUMP 0x10 (DB); F3 = F1 + F2 (slot 1); F4 = F1 * F2 (slot 2); NOP
    let jump_db = [0x063E, 0x0400, 0x0010];
    let prog = [
        &jump_db[..],
        &t2a(31, alu(0x81, 3, 1, 2))[..],
        &t2a(31, mul(4, 1, 2))[..],
        &[0x0001][..],
    ]
    .concat();
    for (simd, big) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut a, mut b) = (prepare(&prog, 9, simd, 0), prepare(&prog, 9, simd, 0));
        if big {
            // F1 = largest float, F2 = 2.0: the add in the first slot stays finite, the multiply
            // in the second overflows. The exception must not end the block early, the slot still
            // has to run `finish_step`
            for c in [&mut a, &mut b] {
                (c.r[1], c.r[2], c.s[1], c.s[2]) =
                    (0x7F7F_FFFF, 0x4000_0000, 0x7F7F_FFFF, 0x4000_0000);
                (c.fx.tag, c.fx_y.tag) = (c.r, c.s);
                c.sys.stky = [0, 0];
            }
        }
        let (block, uops) = block_of(&mut b, prog.len());
        assert_eq!(
            (block.guard, uops.len()),
            (GuardClass::Arith, 3),
            "branch, slot 1, slot 2"
        );
        assert_eq!(
            uops[1].h,
            UopKind::INTERP.0,
            "a float in the first slot is declined"
        );
        assert_eq!(
            UopKind(uops[2].h).family(),
            Some(KindFamily::Float),
            "the second is taken"
        );
        assert_eq!(uops[2].flags & (UF_FIN | UF_LAST), UF_FIN | UF_LAST);
        let ran = run_mini(&mut b, &table, &block, &uops);
        assert_eq!(ran.done, 3, "simd {simd} big {big}");
        for _ in 0..3 {
            a.step().unwrap();
        }
        assert_eq!(b.pc, 0x10, "the delayed branch resolved in the second slot");
        assert_eq!(exact_diff(&a, &b), None, "simd {simd} big {big}");
        if big {
            assert_ne!(
                b.sys.stky,
                [0, 0],
                "the multiply of the second slot overflowed"
            );
            assert_eq!(
                b.irq.due, 0,
                "and poked the controller, like the interpreter"
            );
        }
    }
}

#[test]
fn a_float_instruction_ending_a_static_loop_finishes_the_step_and_loops_back() {
    let table = UopHot::handler_table();
    // R0 = 0; I0 = 0xc0430; DO (+3) UNTIL LCE, LCNTR = 4; then the loop body, the one float
    // instruction F3 = F1 + F2, and a NOP (the head is the program of the translator's loop test)
    let head = [
        0x0F80, 0x0000, 0x0F10, 0x000C, 0x0430, 0x0C00, 0x0480, 0x0003,
    ];
    let prog = [&head[..], &t2a(31, alu(0x81, 3, 1, 2))[..], &[0x0001][..]].concat();
    let body = CODE + head.len() as u32;
    for simd in [false, true] {
        let (mut a, mut b) = (prepare(&prog, 21, simd, 0), prepare(&prog, 21, simd, 0));
        for c in [&mut a, &mut b] {
            for _ in 0..3 {
                c.step().expect("the loop set-up runs");
            }
            assert_eq!(c.pc, body, "the head ends at the loop body");
            c.irq.due = u64::MAX;
        }
        let (block, uops) = block_at(&mut b, body, prog.len());
        assert_eq!(
            (block.guard, uops.len(), uops[0].flags & (UF_FIN | UF_LAST)),
            (GuardClass::Arith, 1, UF_FIN | UF_LAST),
            "a one-uop block ending the loop"
        );
        assert_eq!(UopKind(uops[0].h).family(), Some(KindFamily::Float));
        let mut looped = 0;
        for iteration in 0..4 {
            let ran = run_mini(&mut b, &table, &block, &uops);
            a.step().expect("the interpreter runs the body");
            assert_eq!(ran.done, 1, "simd {simd} iteration {iteration}");
            assert_eq!(
                exact_diff(&a, &b),
                None,
                "simd {simd} iteration {iteration}"
            );
            looped += usize::from(b.pc == body);
        }
        assert_eq!(looped, 3, "three jumps back, the fourth falls through");
        assert_eq!(b.pc, body + 3, "the loop ended on the NOP behind the body");
    }
}

#[test]
fn a_block_of_float_uops_is_arith_guarded() {
    let prog = [t2a(31, alu(0x81, 3, 1, 2)), t2a(31, alu(0x8A, 0, 3, 4))].concat();
    let mut c = core_with(&prog);
    c.irq.due = u64::MAX;
    let (block, _) = block_of(&mut c, prog.len());
    assert_eq!(block.guard, GuardClass::Arith);
    assert_eq!(crate::core::uop::lower::guard::check(&c, &block, 8), None);
    // an arithmetic-exception interrupt that is enabled keeps the block out of the hot loop
    c.mode1 |= 1 << 12;
    (c.imask, c.irq.lirptl) = (u32::MAX, u32::MAX);
    assert!(c.arith_irq_enabled());
    assert_eq!(
        crate::core::uop::lower::guard::check(&c, &block, 8),
        Some(GuardFail::Arith)
    );
}

#[test]
fn float_instructions_translate_to_one_uop_per_step_for_both_modes() {
    let prog = [t2a(31, mul(3, 1, 2)), t2a(5, alu(0xE1, 4, 1, 2))].concat();
    let mut c = core_with(&prog);
    let cfg = UopConfig::default();
    let (blocks, uops) = translate_with(
        &mut c,
        &cfg,
        CODE,
        CODE + 6,
        crate::core::uop::lower::lower_insn,
    );
    assert_eq!(blocks.len(), 2, "one block per PEy mode");
    for b in &blocks {
        let u = &uops[b.first as usize..(b.first + b.len) as usize];
        assert_eq!((b.steps, u.len()), (2, 2));
        let simd_flag = u
            .iter()
            .all(|u| (u.flags & crate::core::uop::types::UF_SIMD != 0) == b.simd);
        assert!(
            simd_flag,
            "UF_SIMD marks the PEy-on variant, simd {}",
            b.simd
        );
        assert_eq!(u[1].b, 5, "the condition code travels in `b`");
    }
}
