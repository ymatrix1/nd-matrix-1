//! Control flow: nested `DO UNTIL` loops (counted and arithmetic), loop ends that are branches,
//! delayed branches with their two slot instructions, calls and returns, indirect jumps and system
//! loads between fast instructions.
//!
//! OWNER: W3-FUZZ. These are the shapes where the block-end protocol (P1, P2) can drift: a loop
//! end as a block-final uop with `UF_FIN`, the countdown of a delayed branch inside the handler,
//! the loop back-edge of a terminator that is also the last instruction of a body, a `Jump` exit
//! into the middle of another block. All targets are instruction starts of the program
//! (`shapes::control`), the cuts move with the block limit drawn per case.
use super::driver::sweep;
use super::scenario::Scenario;
use super::shapes::{control, do_loop, insn, loops, straight, vary_cfg, ALL, REGS};
use super::sieve::Fam;
use super::state::Prep;
use crate::core::uop::exec::h_flow::enc::{self, TRUE};
use crate::core::uop::verify::gen::{Rng, CODE_PC};

/// Cases per test at the CI bound. ASSUMED: 30 programs of up to 160 steps per shape keep this
/// file near 2 s in the debug profile.
const CASES: u64 = 30;

/// The state of a control case: stale extensions (the bodies move float registers), a drawn PEy
/// mode, hidden state half of the time, the DAG only when bodies touch memory.
fn prep(rng: &mut Rng, fams: &[Fam]) -> Prep {
    Prep {
        simd: rng.below(2) == 1,
        stale: true,
        hidden: rng.below(2) == 1,
        dag: fams.contains(&Fam::Move),
        stack: rng.below(3) as usize,
        ..Prep::default()
    }
}

/// A case over `program` whose indirect jumps and returns are aimed at `starts`.
fn scenario(
    name: &'static str,
    rng: &mut Rng,
    seed: u64,
    fams: &[Fam],
    program: Vec<u16>,
    starts: Vec<u32>,
) -> Scenario {
    let steps = 40 + u64::from(rng.below(160));
    let mut s = Scenario::new(name, seed, program, steps);
    s.prep = prep(rng, fams);
    s.cfg = vary_cfg(rng);
    s.targets = starts;
    // a branch with slots that always bails to the interpreter keeps a case vacuous: count the
    // cases together (half must run a fast uop)
    s.heavy = name == "delayed";
    s
}

#[test]
fn jumps_calls_returns_and_delayed_branches_in_register_code_equal_the_interpreter() {
    let t = sweep("control-regs", 20, CASES, |rng, seed| {
        let (p, starts) = {
            let n = 6 + rng.below(14) as usize;
            control(rng, &REGS, n)
        };
        scenario("control-regs", rng, seed, &REGS, p, starts)
    });
    assert!(
        t.exits[1] > 0,
        "no `Jump` exit over {} cases: {t:?}",
        t.cases
    );
}

#[test]
fn the_same_shapes_with_memory_moves_in_the_bodies_equal_the_interpreter() {
    // moves through DAG2 registers that the indirect jumps aim at code: some stores land in the
    // program (self-modifying code), which both engines must see alike
    sweep("control-all", 21, CASES, |rng, seed| {
        let (p, starts) = {
            let n = 6 + rng.below(14) as usize;
            control(rng, &ALL, n)
        };
        scenario("control-all", rng, seed, &ALL, p, starts)
    });
}

#[test]
fn nested_counted_and_arithmetic_loops_equal_the_interpreter() {
    sweep("loops", 22, CASES, |rng, seed| {
        let fams: &[Fam] = if rng.below(2) == 0 { &REGS } else { &ALL };
        let p = loops(rng, fams);
        scenario("loops", rng, seed, fams, p, vec![CODE_PC])
    });
}

/// Appends one instruction and records its start.
fn put(p: &mut Vec<u16>, starts: &mut Vec<u32>, i: Vec<u16>) {
    starts.push(CODE_PC + p.len() as u32);
    p.extend(i);
}

/// A delayed branch of `kind` (0 jump, 1 call, 2 return) between fast instructions: a head that
/// always runs, the branch with two slot instructions, and a tail of instructions to land on.
fn delayed(rng: &mut Rng, fams: &[Fam]) -> (Vec<u16>, Vec<u32>) {
    let mut p = Vec::new();
    let mut starts = Vec::new();
    for _ in 0..1 + rng.below(3) {
        put(&mut p, &mut starts, insn(rng, fams));
    }
    let branch_at = CODE_PC + p.len() as u32;
    let tail_len: u32 = 5;
    // the targets: a head instruction (a loop), or the tail; the tail starts are drawn first
    // from the head list and fixed up after the layout is known
    let cond = if rng.below(2) == 0 {
        TRUE
    } else {
        rng.below(32) as u8
    };
    let (slot1, slot2) = (insn(rng, fams), insn(rng, fams));
    let kind = rng.below(3);
    let len = [3, 3, 1][kind as usize] + (slot1.len() + slot2.len()) as u32;
    let tail_pc = branch_at + len;
    let target = if rng.below(3) == 0 {
        starts[rng.below(starts.len() as u32) as usize]
    } else {
        tail_pc
    };
    let mut branch = match kind {
        0 => enc::jump(cond, target, false, true),
        1 => enc::jump(cond, target, true, true),
        _ => enc::rts(cond, true, false),
    };
    branch.extend(&slot1);
    branch.extend(&slot2);
    starts.push(branch_at);
    p.extend(branch);
    for _ in 0..tail_len {
        put(&mut p, &mut starts, insn(rng, fams));
    }
    (p, starts)
}

#[test]
fn delayed_branches_and_their_slots_equal_the_interpreter() {
    let t = sweep("delayed", 23, CASES, |rng, seed| {
        let fams: &[Fam] = if rng.below(3) == 0 { &ALL } else { &REGS };
        let (p, starts) = delayed(rng, fams);
        scenario("delayed", rng, seed, fams, p, starts)
    });
    assert!(t.steps_uop > t.steps_interp_uop, "{t:?}");
}

/// A counted loop whose last body instruction is a branch: a jump back into the body, a jump out
/// of the loop, a call, a return or an indirect jump (the terminator-at-loop-end contract: the
/// handler takes the back-edge itself, the translator puts no `UF_FIN` on it).
fn terminated_loop(rng: &mut Rng, fams: &[Fam]) -> (Vec<u16>, Vec<u32>) {
    let head = insn(rng, fams);
    let (b1, b2) = (insn(rng, fams), insn(rng, fams));
    let body_start = CODE_PC + head.len() as u32 + 3;
    let end_off = 3 + (b1.len() + b2.len()) as u32;
    let term_pc = body_start + (b1.len() + b2.len()) as u32;
    let cond = if rng.below(2) == 0 {
        TRUE
    } else {
        rng.below(32) as u8
    };
    let after = term_pc + 3;
    let to = if rng.below(2) == 0 { body_start } else { after };
    let term = match rng.below(5) {
        0 => enc::jump(cond, to, false, false),
        1 => enc::jump(cond, to, true, false),
        2 => enc::jump_i(cond, 8, 9, false, false),
        3 => enc::jump_pc6(cond, 4, false, false),
        _ => enc::rts(cond, false, false),
    };
    let mut p = head;
    p.extend(enc::do_count(1 + rng.below(3) as u16, end_off));
    p.extend(b1);
    p.extend(b2);
    p.extend(term);
    p.extend(straight(rng, fams, 4));
    (p, vec![CODE_PC, body_start, after])
}

#[test]
fn a_branch_that_is_the_last_instruction_of_a_loop_body_equals_the_interpreter() {
    sweep("loop-end-branch", 24, CASES, |rng, seed| {
        let (p, starts) = terminated_loop(rng, &REGS);
        scenario("loop-end-branch", rng, seed, &REGS, p, starts)
    });
}

#[test]
fn a_deeply_nested_loop_stack_equals_the_interpreter() {
    // depth 3 to 5: the loop stack, the nested ends and the budget of the blocks inside
    sweep("deep-loops", 25, CASES / 2, |rng, seed| {
        let depth = 3 + rng.below(3);
        let mut p = straight(rng, &REGS, 1);
        p.extend(do_loop(rng, &REGS, depth));
        p.extend(straight(rng, &REGS, 3));
        scenario("deep-loops", rng, seed, &REGS, p, vec![CODE_PC])
    });
}
