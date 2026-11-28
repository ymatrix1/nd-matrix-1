//! Differential tests of the lowering against `Core::step`, the only exactness oracle.
//!
//! OWNER: W1-LOWER. Two questions, both answered by running the interpreter:
//!
//! 1. Is every "no" of the cut rules true? An instruction that `cut.rs` calls plain (no cut, no
//!    arithmetic-exception class) must leave the guard-relevant state and the control flow
//!    alone, whatever its operands. The corpus is the census forms, the shared program pool, one
//!    instruction per terminator, a sweep of every register code of the immediate loads,
//!    bit-flipped census forms and random words; every counter below proves that the interesting
//!    cases happened (a vacuous run fails).
//! 2. Do the translated blocks describe what the interpreter does? Random programs with
//!    conditional branches (taken and not, delayed and not), DO loops, system-register loads and
//!    a random `stop_pc` run in lockstep on `Core::step` and on the blocks (`support::lockstep`,
//!    which executes the instruction of every uop with `exec_cached`): same pc chain, the static
//!    budget equals the interpreter's steps, no interior instruction leaves the chain or lands on
//!    `stop_pc`, and `exact_diff` finds the cores identical after every block.
use super::cut::{can_raise_arith, flow_of, touches_sys, translatable, Flow};
use super::decode::decode_at;
use super::lower_insn;
use super::support::{build, guard_state, lockstep, reg_move, Seen, FLOW_CASES};
use crate::core::decode_cache::{DecodedOp, MoveOp};
use crate::core::support::{core_with, CODE};
use crate::core::uop::config::UopConfig;
use crate::core::uop::verify::compare::exact_diff;
use crate::core::uop::verify::gen::{fill_state, Rng, PROGRAM_POOL};
use std::collections::BTreeSet;

const CENSUS: &str = include_str!("../../../../../../docs/research/sharc-seq/census_cases.txt");
const EXTRA: &str =
    include_str!("../../../../../../docs/research/sharc-seq/census_extra_cases.txt");

/// The distinct instruction words of the census, as parcels.
fn census_forms() -> Vec<Vec<u16>> {
    let tokens: BTreeSet<&str> = CENSUS
        .lines()
        .chain(EXTRA.lines())
        .filter_map(|l| l.split_whitespace().next())
        .filter(|t| matches!(t.len(), 4 | 8 | 12) && t.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    tokens
        .into_iter()
        .map(|t| {
            (0..t.len() / 4)
                .map(|k| u16::from_str_radix(&t[4 * k..4 * k + 4], 16).unwrap())
                .collect()
        })
        .collect()
}

/// Source codes of the register-move sweep: R0, R15, I0, M0, L0, I15 and the top system code.
const REG_MOVE_SOURCES: [u8; 7] = [0x00, 0x0F, 0x10, 0x18, 0x20, 0x37, 0x7F];

/// Every instruction shape the classification is checked on.
fn corpus(rng: &mut Rng) -> Vec<Vec<u16>> {
    let mut all = census_forms();
    all.extend(PROGRAM_POOL.iter().map(|p| p.to_vec()));
    all.extend(FLOW_CASES.iter().map(|(_, p, _)| p.to_vec()));
    for u in 0..=0x7Fu16 {
        all.extend([0, 0x1234, 0xFFFF].map(|imm| vec![0x0F80 | u, imm]));
        all.push(vec![0x0F00 | u, 0xDEAD, 0xBEEF]);
    }
    // register moves (`ureg = ureg`, T5a and T5b): every destination code against data, DAG and
    // system sources, so a move into MODE1, IMASK, the timers, ... is on the corpus
    for dst in 0..=0x7Fu8 {
        for src in REG_MOVE_SOURCES {
            all.extend([false, true].map(|short| reg_move(src, dst, short)));
        }
    }
    let known = all.clone();
    for form in &known {
        for _ in 0..4 {
            let mut m = form.clone();
            m.resize(3, 0);
            for _ in 0..=rng.below(3) {
                let bit = rng.below(48) as usize;
                m[bit / 16] ^= 1 << (bit % 16);
            }
            all.push(m);
        }
    }
    all.extend((0..6000).map(|_| (0..3).map(|_| rng.next_u32() as u16).collect::<Vec<u16>>()));
    all
}

#[derive(Debug, Default)]
struct Tally {
    run: u32,
    faulted: u32,
    skipped: u32,
    /// Guard state changed, explained by a system-register load.
    sys_changed: u32,
    /// Guard state changed, explained by an arithmetic exception.
    arith_changed: u32,
    /// The pc or the sequencer left the straight line, explained by a flow instruction.
    flow_changed: u32,
    /// A delayed branch left a pending branch.
    pending: u32,
    /// Register moves into a system register that changed the guard state.
    reg_move_sys_changed: u32,
    /// Plain instructions that changed nothing: the "no" side, checked.
    unchanged: u32,
}

/// Runs `program` once per PEy mode on random states and checks the classification of its first
/// instruction against what the interpreter did.
fn classify_case(program: &[u16], rng: &mut Rng, t: &mut Tally) -> Result<(), String> {
    for simd in [false, true] {
        let mut c = core_with(program);
        fill_state(&mut c, rng, simd);
        c.irq.due = u64::MAX;
        let pc = c.pc;
        let insn = decode_at(&mut c, pc);
        if !translatable(&insn) {
            t.skipped += 1;
            continue;
        }
        let before = guard_state(&c);
        if c.step().is_err() {
            t.faulted += 1;
            continue;
        }
        t.run += 1;
        let state_changed = guard_state(&c) != before;
        let flow_changed = c.pc != insn.next_pc
            || c.pending.is_some()
            || !c.loops.is_empty()
            || !c.call_stack.is_empty();
        let flow = flow_of(&insn);
        let (sys, arith) = (touches_sys(&insn), can_raise_arith(&insn));
        let name = format!("{program:04x?} simd={simd}");
        if flow_changed && flow == Flow::Plain {
            return Err(format!(
                "{name}: the pc or sequencer moved but the flow class is Plain"
            ));
        }
        if state_changed && flow == Flow::Plain && !sys && !arith {
            return Err(format!(
                "{name}: guard state changed, no cut and no arith class"
            ));
        }
        if c.pending.is_some() && flow != (Flow::End { slots: 2 }) {
            return Err(format!("{name}: a branch is pending, flow class {flow:?}"));
        }
        let into_sys =
            matches!(insn.op, DecodedOp::Move(MoveOp::RegMove { dst, .. }) if dst >= 0x60);
        t.reg_move_sys_changed += u32::from(into_sys && state_changed);
        t.pending += u32::from(c.pending.is_some());
        t.flow_changed += u32::from(flow_changed);
        if state_changed && flow == Flow::Plain {
            t.sys_changed += u32::from(sys);
            t.arith_changed += u32::from(arith && !sys);
        }
        t.unchanged +=
            u32::from(!state_changed && !flow_changed && flow == Flow::Plain && !sys && !arith);
    }
    Ok(())
}

#[test]
fn a_plain_nop_leaves_the_guard_state_alone_so_the_comparison_can_see_a_change() {
    let mut c = core_with(&[0x0001]);
    c.irq.due = u64::MAX;
    let before = guard_state(&c);
    c.step().unwrap();
    assert_eq!(guard_state(&c), before);
    // and it does see a system-register write that flips PEyen (MODE1 is universal register 0x79?
    // any load into a register code >= 0x60 that changes a compared field will do)
    let mut seen_change = false;
    for u in 0x60u16..=0x7F {
        let mut c = core_with(&[0x0F80 | u, 0xFFFF]);
        c.irq.due = u64::MAX;
        let before = guard_state(&c);
        seen_change |= c.step().is_ok() && guard_state(&c) != before;
    }
    assert!(
        seen_change,
        "no system-register load changed the compared state"
    );
}

#[test]
fn no_instruction_classified_plain_changes_guarded_state_or_leaves_the_straight_line() {
    let mut rng = Rng::new(0x1ee7_c0de);
    let mut t = Tally::default();
    for program in corpus(&mut rng) {
        if let Err(e) = classify_case(&program, &mut rng, &mut t) {
            panic!("{e}");
        }
    }
    eprintln!("classification corpus: {t:?}");
    assert!(t.run >= 8000, "{t:?}");
    assert!(
        t.sys_changed >= 40,
        "system-register loads that moved the guard state: {t:?}"
    );
    assert!(
        t.arith_changed >= 40,
        "arithmetic exceptions that moved the guard state: {t:?}"
    );
    assert!(
        t.reg_move_sys_changed >= 100,
        "register moves into system registers that moved the guard state: {t:?}"
    );
    assert!(t.flow_changed >= 300 && t.pending >= 30, "{t:?}");
    assert!(
        t.unchanged >= 3000,
        "the plain side must be exercised in volume: {t:?}"
    );
}

/// Position-independent snippets: plain instructions, forward conditional jumps (delayed or
/// not), DO loops with a one- and a two-instruction body, system-register loads.
fn snippet(rng: &mut Rng) -> Vec<u16> {
    const ALU: [u16; 3] = [0x013e, 0x0000, 0x1312];
    match rng.below(10) {
        0..=3 => PROGRAM_POOL[rng.below(PROGRAM_POOL.len() as u32) as usize].to_vec(),
        4 | 5 => {
            // IF cond JUMP (pc + k) [(DB)], relative
            let cond = rng.below(32) as u16;
            let delay = if rng.below(2) == 0 { 0x0400 } else { 0 };
            vec![0x0700 | (cond << 1), delay, 3 + rng.below(9) as u16]
        }
        6 => [vec![0x0C00, 0x0300, 0x0004], vec![0x0001, 0x0001]].concat(),
        7 => [vec![0x0C00, 0x0200, 0x0003], ALU.to_vec()].concat(),
        8 => vec![0x0F80 | 0x60 | rng.below(32) as u16, rng.next_u32() as u16],
        _ => vec![0x0001],
    }
}

fn random_program(rng: &mut Rng) -> Vec<u16> {
    (0..8 + rng.below(20)).flat_map(|_| snippet(rng)).collect()
}

fn run_program(
    program: &[u16],
    seed: u64,
    simd: bool,
    stop_pc: Option<u32>,
) -> Result<Seen, String> {
    let cfg = UopConfig::default();
    let (mut b, built) = build(program, &cfg, stop_pc, lower_insn);
    let mut a = core_with(program);
    for c in [&mut a, &mut b] {
        fill_state(c, &mut Rng::new(seed), simd);
        c.irq.due = u64::MAX;
    }
    if let Some(d) = exact_diff(&a, &b) {
        return Err(format!("the two cores start different: {d:?}"));
    }
    lockstep(&mut a, &mut b, &built, stop_pc, 150)
}

#[test]
fn translated_blocks_follow_the_interpreter_with_the_static_budget_on_random_programs() {
    let mut rng = Rng::new(0xb10c_5eed);
    let mut total = Seen::default();
    for k in 0..1200 {
        let program = random_program(&mut rng);
        let stop_pc = (k % 3 == 0).then(|| CODE + rng.below(program.len() as u32));
        let (seed, simd) = (rng.next_u64(), k % 2 == 1);
        match run_program(&program, seed, simd, stop_pc) {
            Ok(s) => {
                total.blocks += s.blocks;
                total.steps += s.steps;
                total.taken += s.taken;
                total.fell_through += s.fell_through;
                total.groups += s.groups;
                total.loop_backs += s.loop_backs;
            }
            Err(e) => panic!("program {k} {program:04x?} stop {stop_pc:x?}: {e}"),
        }
    }
    eprintln!("lockstep: {total:?}");
    assert!(total.blocks >= 2000 && total.steps >= 5000, "{total:?}");
    assert!(total.taken >= 100, "taken branches: {total:?}");
    assert!(
        total.fell_through >= 500,
        "blocks that fell through: {total:?}"
    );
    assert!(total.groups >= 60, "delay groups executed: {total:?}");
    assert!(total.loop_backs >= 100, "loop back-edges: {total:?}");
}

#[test]
fn every_block_of_a_random_program_is_well_formed_and_the_two_modes_agree() {
    let mut rng = Rng::new(0x7411_0000);
    let mut checked = 0;
    for _ in 0..200 {
        let program = random_program(&mut rng);
        let stop = Some(CODE + rng.below(program.len() as u32)).filter(|_| rng.below(2) == 0);
        let (core, built) = build(&program, &UopConfig::default(), stop, lower_insn);
        let shape = |simd| {
            built
                .mode(simd)
                .iter()
                .map(|(b, u)| (b.pc, b.end_pc, b.steps, u.len()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shape(false),
            shape(true),
            "PEy off and on are translated alike"
        );
        let mut floor = CODE;
        for (b, uops) in built.mode(false) {
            assert!(
                b.pc >= floor && b.end_pc > b.pc,
                "blocks are ordered and disjoint"
            );
            floor = b.end_pc;
            assert_eq!(b.last_pc, uops.last().unwrap().pc);
            assert_eq!(b.steps as usize, uops.len(), "all-Interp: one step per uop");
            assert_eq!(b.cum.len(), uops.len());
            assert!(uops
                .windows(2)
                .all(|w| w[0].pc < w[1].pc && w[0].b == w[1].pc));
            let last = uops
                .iter()
                .filter(|u| u.flags & crate::core::uop::types::UF_LAST != 0);
            assert_eq!(last.count(), 1, "UF_LAST on the last uop only");
            assert_eq!(b.code_hash, super::decode::code_hash(&core, b.pc, b.end_pc));
            let interior = uops[1..].iter().any(|u| Some(u.pc) == stop);
            assert!(!interior, "stop_pc {stop:x?} inside block {:#x}", b.pc);
            checked += 1;
        }
    }
    assert!(checked >= 800, "{checked} blocks checked");
}

#[test]
fn enable_uop_over_a_synthetic_image_translates_it_and_keeps_every_instruction_interp() {
    let mut rng = Rng::new(0xe4ab_1e00);
    let program = random_program(&mut rng);
    let mut core = core_with(&program);
    let hi = CODE + program.len() as u32;
    let report = core
        .enable_uop(UopConfig::default(), &[(CODE, hi)])
        .unwrap();
    assert!(
        report.blocks >= 4 && report.blocks.is_multiple_of(2),
        "{report:?}"
    );
    assert!(
        report.insns >= 8 && report.insns == report.interp_insns,
        "{report:?}"
    );
    assert_eq!(report.uops, report.insns, "one uop per instruction");
    let hot = core.uop.as_ref().unwrap();
    for simd in [false, true] {
        let entry = hot
            .pcmap(simd)
            .get(CODE)
            .expect("the first instruction has a block");
        assert_eq!(hot.blocks[entry.0 as usize].pc, CODE);
        assert_eq!(hot.blocks[entry.0 as usize].simd, simd);
    }
}
