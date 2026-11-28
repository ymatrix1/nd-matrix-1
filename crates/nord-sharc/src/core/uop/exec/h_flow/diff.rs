//! Block-level differential tests of the flow handlers: one flow instruction and the code around
//! it, run as a block through the real handler table next to `Core::step`, on random states.
//!
//! OWNER: W2-FLOW. Recipe P8 with the rig of `harness.rs` (the translator lowers, the handlers
//! run, `exact_diff` compares everything). Every test asserts that the handlers ran (P10): a
//! differential run that lowers nothing passes trivially, so the counts of the executor are
//! checked against the number of cases and the exits that must have happened.
//!
//! The state of the flow domain (P13) is filled here: the PC stack, the DAG2 registers of the
//! indirect jumps, up to two loop frames (their end at the branch, at a delay slot or far away,
//! which is how the terminator-at-a-loop-end contract is exercised) and an interrupt in service.
use super::enc::{self, NOP, TRUE};
use super::harness::Tally;
use super::rig::Rig;
use super::{DO_COUNT, DO_UNTIL, IDLE, JUMP, JUMP_I, RET, SYS_LOAD};
use crate::core::decode_cache::{DecodedOp, MoveOp};
use crate::core::interrupts::Serviced;
use crate::core::support::{core_with, CODE};
use crate::core::uop::config::UopConfig;
use crate::core::uop::lower::builder::BlockBuilder;
use crate::core::uop::lower::decode::decode_at;
use crate::core::uop::lower::flow::FlowFamily;
use crate::core::uop::lower::{Family, LowerCtx};
use crate::core::uop::types::{Exit, Uop};
use crate::core::uop::verify::gen::{census_encodings, fill_stacks, fill_state, Rng};
use crate::core::{Core, LoopFrame};

/// Seeds per instruction and mode. ASSUMED: three random states per case keep the grid under
/// half a minute; the per-kind call counts the tests assert show what they reached.
const SEEDS: u64 = 3;
/// Parcels after the instruction under test: the delay slots and the code a fall-through or a
/// loop runs into.
const TAIL: [u16; 6] = [NOP; 6];

/// Fills the registers, flags, the PC stack, DAG2, loop frames and the interrupt service of `c`
/// from `seed`. `len` is the length of the instruction at `CODE` in parcels.
fn fill_flow(c: &mut Core, seed: u64, simd: bool, len: u32) {
    let mut rng = Rng::new(seed);
    fill_state(c, &mut rng, simd);
    for k in 8..16 {
        c.i[k] = CODE + rng.below(0x60);
        c.m[k] = rng.below(4);
    }
    let depth = rng.pick(&[0usize, 0, 1, 2, 5, 28, 29, 30]);
    fill_stacks(c, &mut rng, depth, 0);
    // a frame that ends at the instruction itself, at a delay slot, or nowhere near
    let ends = [CODE, CODE + len, CODE + len + 1, CODE + 0x200];
    for _ in 0..rng.pick(&[0, 0, 1, 1, 2]) {
        c.loops.push(LoopFrame {
            start: CODE + 0x100,
            end: rng.pick(&ends),
            remaining: rng.pick(&[1, 2, 3, 9]),
            cs_depth: c.call_stack.len(),
            ..LoopFrame::default()
        });
    }
    if rng.below(3) == 0 {
        let status_pushed = rng.below(2) == 0;
        c.irq.serviced.push(Serviced {
            vector: 6,
            status_pushed,
        });
        if status_pushed {
            c.sys.status.push([rng.next_u32(), rng.next_u32(), c.mode1]);
        }
    }
}

/// Runs `program` on random states of every seed and both PEy modes for up to `iters` blocks or
/// steps each, adds the counts to `total` and panics with the case on the first difference.
fn check(name: &str, program: &[u16], len: u32, iters: u32, total: &mut Tally) {
    check_seeds(name, program, len, iters, SEEDS, total);
}

/// [`check`] with `seeds` seeds.
fn check_seeds(name: &str, program: &[u16], len: u32, iters: u32, seeds: u64, total: &mut Tally) {
    let program = [program, &TAIL].concat();
    for seed in 0..seeds {
        for simd in [false, true] {
            let mut rig = Rig::new(&program, |c| fill_flow(c, seed, simd, len));
            rig.run(iters)
                .unwrap_or_else(|e| panic!("{name} {program:04x?} seed {seed} simd {simd}: {e}"));
            total.merge(&rig.tally);
        }
    }
}

/// Every flow form with every condition code, in the shapes the translator lowers. The conditions
/// that read a different flag (`EQ`, `NE`, `LT`, `LCE`, `TRUE`) get every combination of the
/// call, delay and interrupt-return bits, the others the two corners.
fn grid() -> Vec<(String, Vec<u16>)> {
    let mut v = Vec::new();
    let all = [(false, false), (true, false), (false, true), (true, true)];
    for cond in 0..32u8 {
        let flags = if matches!(cond, 0 | 1 | 4 | 5 | 16 | 31) {
            &all[..]
        } else {
            &[all[0], all[3]]
        };
        for &(call, delayed) in flags {
            let tag = format!("c{cond} call={call} db={delayed}");
            let (ri, rm) = (8 + cond % 8, 8 + cond / 4);
            v.push((
                format!("jump {tag}"),
                enc::jump(cond, CODE + 0x40, call, delayed),
            ));
            v.push((
                format!("jump in range {tag}"),
                enc::jump(cond, CODE + 8, call, delayed),
            ));
            v.push((
                format!("jump rel {tag}"),
                enc::jump_rel(cond, 6, call, delayed),
            ));
            v.push((
                format!("jump (m,i) {tag}"),
                enc::jump_i(cond, ri, rm, call, delayed),
            ));
            v.push((
                format!("jump pc6 {tag}"),
                enc::jump_pc6(cond, -3, call, delayed),
            ));
            v.push((format!("rts {tag}"), enc::rts(cond, delayed, call)));
            v.push((format!("ret {tag}"), enc::ret_wide(cond, delayed, call)));
        }
        v.push((format!("do until c{cond}"), enc::do_until(cond, 5)));
    }
    for (count, off) in [(1u16, 3u32), (2, 4), (3, 5), (300, 6)] {
        v.push((format!("do {count} end +{off}"), enc::do_count(count, off)));
    }
    for ureg in [0u8, 1, 3, 9, 15] {
        v.push((format!("do r{ureg}"), enc::do_reg(ureg, 5)));
    }
    v.push(("idle".into(), enc::idle(false)));
    v.push(("idle wide".into(), enc::idle(true)));
    v
}

#[test]
fn every_flow_form_matches_the_interpreter_on_random_states_and_the_handlers_ran() {
    let mut total = Tally::default();
    let cases = grid();
    for (name, p) in &cases {
        check(name, p, p.len() as u32, 3, &mut total);
    }
    // every kind ran on a good part of its cases
    for k in [JUMP, JUMP_I, RET, DO_COUNT, DO_UNTIL] {
        assert!(total.calls(k) >= 16, "{k:?} ran {} times", total.calls(k));
    }
    // IDLE has two forms, each on every seed and mode, and each case runs it once
    let idle_cases = 2 * 2 * SEEDS as u32;
    assert!(
        total.calls(IDLE) >= idle_cases,
        "IDLE ran {} times in {idle_cases} cases",
        total.calls(IDLE)
    );
    // the bail exits (`Fault`, `Slow`) need a state the random fill rarely makes: the next test
    for e in [Exit::Ok, Exit::Jump] {
        assert!(total.exit(e) > 0, "no uop exited {e:?}");
    }
    assert!(
        total.fin > 0 && total.fin_jumps > 0,
        "no delay slot resolved a branch: {total:?}"
    );
    assert!(total.blocks > 0 && total.stepped > 0, "{total:?}");
}

#[test]
fn from_the_whole_census_the_family_takes_only_flow_forms_and_system_loads() {
    let all = census_encodings(|_, _| true);
    assert!(all.len() > 250, "{} census encodings", all.len());
    let (mut total, mut taken) = (Tally::default(), 0);
    for p in &all {
        let mut core = core_with(p);
        let c = decode_at(&mut core, CODE);
        let cfg = UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd: false,
            isa: false,
        };
        let mut b = BlockBuilder::new(CODE, false, false);
        let took = FlowFamily::lower(&cx, &c, &mut b);
        let sys_load =
            matches!(c.op, DecodedOp::Move(MoveOp::LoadImm { ureg, .. }) if ureg >= 0x60);
        let flow = matches!(
            c.op,
            DecodedOp::Flow(_) | DecodedOp::DoUntil | DecodedOp::Idle
        );
        assert!(!took || sys_load || flow, "{p:04x?} taken: {:?}", c.op);
        assert_eq!(
            took && sys_load,
            b.uops().first().is_some_and(|u| u.h == SYS_LOAD.0)
        );
        taken += u32::from(took);
        // a census instruction the family declines is an `Interp` uop (exact by construction,
        // and a store may rewrite the code the block was built from: the dispatch loop's `Smc`
        // exit, not this test), so only what the family took is run
        if took {
            check_seeds("census", p, p.len() as u32, 1, 2, &mut total);
        }
    }
    assert!(
        taken > 0 && total.calls(SYS_LOAD) > 0,
        "{taken} taken: {total:?}"
    );
}

#[test]
fn a_system_register_load_matches_the_interpreter_for_every_register_code() {
    for code in 0..0x80u8 {
        let mut total = Tally::default();
        for value in [0u32, 0xFFFF_FFFF, 0xC_0430, 0x8000_0000] {
            let p = enc::load_imm(code, value);
            let name = format!("ureg {code:#x} = {value:#x}");
            check_seeds(&name, &p, p.len() as u32, 2, 2, &mut total);
        }
        // exactly the system registers are lowered; a frame ending at the load keeps some runs
        // on the interpreter, so the count is positive, not the number of runs
        assert_eq!(
            total.calls(SYS_LOAD) > 0,
            code >= 0x60,
            "ureg {code:#x}: {total:?}"
        );
    }
}

#[test]
fn a_system_register_load_in_the_second_delay_slot_is_finished_by_the_dispatch_loop() {
    let mut total = Tally::default();
    for code in 0x60..0x80u8 {
        for dst in [CODE + 20, CODE + 9] {
            let mut p = enc::jump(TRUE, dst, code & 1 == 0, true);
            p.push(NOP);
            p.extend(enc::load_imm(code, 0x30 + u32::from(code)));
            check(&format!("slot ureg {code:#x}"), &p, 3, 3, &mut total);
        }
    }
    assert!(
        total.calls(SYS_LOAD) > 0 && total.calls(JUMP) > 0,
        "{total:?}"
    );
    assert!(total.fin_jumps > 0, "the slot resolved the branch");
}

#[test]
fn a_handler_that_cannot_run_leaves_the_instruction_to_the_interpreter() {
    let mut total = Tally::default();
    // RTS and RTI with nothing on the PC stack, DO with a zero count register
    for (name, p) in [
        ("rts", enc::rts(TRUE, false, false)),
        ("rti", enc::ret_wide(TRUE, false, true)),
        ("do r2", enc::do_reg(2, 4)),
    ] {
        let mut rig = Rig::new(&[&p[..], &TAIL].concat(), |c| {
            c.r[2] = 0;
            c.call_stack.clear();
        });
        assert!(
            !rig.advance().unwrap_or_else(|e| panic!("{name}: {e}")),
            "{name} faults on both"
        );
        assert_eq!(rig.tally.exit(Exit::Fault), 1, "{name}: {:?}", rig.tally);
        total.merge(&rig.tally);
    }
    // IDLE with a peripheral model attached: the handler declines and `Core::step` sleeps both
    let mut rig = Rig::new(&[&enc::idle(false)[..], &TAIL].concat(), |c| {
        c.periph = Some(Box::default())
    });
    rig.advance().unwrap();
    assert_eq!((rig.tally.calls(IDLE), rig.tally.exit(Exit::Slow)), (1, 1));
    assert!(rig.a.is_idle() && rig.b.is_idle());
    assert_eq!(total.calls(RET) + total.calls(DO_COUNT), 3);
}

/// A wrong operand: the case name, the program and the mutation of its uops.
type Mutation = (&'static str, Vec<u16>, fn(&mut Uop));

#[test]
fn the_comparison_notices_a_wrong_operand_in_every_family_kind() {
    let cases: [Mutation; 4] = [
        ("call bit", enc::jump(TRUE, CODE + 0x40, true, false), |u| {
            u.b &= !super::pack::CALL
        }),
        ("target", enc::jump(TRUE, CODE + 0x40, false, false), |u| {
            u.a ^= 1
        }),
        ("count", enc::do_count(3, 4), |u| u.b += 1),
        ("length", enc::jump(TRUE, CODE + 0x40, true, false), |u| {
            u.b ^= 1 << 8
        }),
    ];
    for (name, p, mutate) in cases {
        let mut caught = 0;
        for seed in 0..SEEDS {
            let mut rig = Rig::new(&[&p[..], &TAIL].concat(), |c| fill_flow(c, seed, false, 3));
            for u in rig.uops.iter_mut().filter(|u| u.h >= 0x400) {
                mutate(u);
            }
            caught += u32::from(rig.advance().is_err());
        }
        assert!(
            caught > 0,
            "{name}: a wrong operand went unnoticed in {SEEDS} seeds"
        );
    }
}
