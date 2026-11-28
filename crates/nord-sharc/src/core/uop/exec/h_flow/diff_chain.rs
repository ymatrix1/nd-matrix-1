//! Differential tests of whole programs: several blocks, run one after the other through the
//! translator, the real handlers and the entry guard, next to `Core::step`.
//!
//! OWNER: W2-FLOW. What one block cannot show: nested `DO` loops, a terminator that is also the
//! last instruction of a loop body (the inherited P1 item: it gets no `UF_FIN`, its handler takes
//! the back-edge itself), delayed branches whose slots carry `UF_FIN`, calls and returns that
//! nest, conditional and indirect jumps. The events (`IDLE`, interrupts, system-register loads)
//! and the random programs are in `diff_events.rs`. Each test asserts what ran (P10): block and
//! handler counts, and that no block was refused where the program is built to be accepted.
use super::enc::{self, Asm, NOP, TRUE};
use super::harness::Tally;
use super::rig::Rig;
use super::{DO_COUNT, JUMP, JUMP_I, KINDS, RET, SYS_LOAD};
use crate::core::support::CODE;
use crate::core::uop::types::{Exit, Uop, UopCtx, UF_FIN};

/// A loop `DO (PC, end) UNTIL LCE, count` whose body is `NOP` and `last`, the last instruction
/// of the body. Returns the program and the pc after it.
fn loop_ending_in(last: &[u16], count: u16) -> (Vec<u16>, u32) {
    let mut a = Asm::default();
    let off = 3 + 1; // the end is the instruction after the NOP
    a.put(&enc::do_count(count, off));
    a.put(&[NOP]);
    a.put(last);
    let after = a.pc();
    a.nops(16);
    (a.0, after)
}

#[test]
fn nested_counted_loops_run_as_blocks_to_the_end_and_match_the_interpreter() {
    let mut a = Asm::default();
    a.put(&enc::do_count(3, 9)); // outer: body 3..=9
    a.put(&[NOP]);
    a.put(&enc::do_count(2, 4)); // inner at 4: body 7..=8
    a.nops(4);
    a.nops(8);
    let mut rig = Rig::new(&a.0, |c| c.irq.due = u64::MAX);
    rig.run_to(CODE + 10, 100).unwrap();
    assert_eq!(
        rig.b.steps,
        1 + 3 * (1 + 1 + 4 + 1),
        "three passes of NOP, DO, 2 x (NOP, NOP), NOP"
    );
    assert!(rig.b.loops.is_empty());
    assert_eq!(rig.tally.calls(DO_COUNT), 4, "one outer, three inner");
    assert_eq!(rig.tally.stepped, 0, "every block was let in");
    assert!(
        rig.tally.fin >= 9,
        "the loop ends are block-final with UF_FIN: {:?}",
        rig.tally
    );
}

/// The terminators the loop-end contract covers, as the last instruction of a loop body.
fn terminators_at_a_loop_end() -> Vec<(&'static str, Vec<u16>)> {
    let to = CODE + 12;
    let (ne, eq) = (1, 0); // NE and EQ
    vec![
        ("jump", enc::jump(TRUE, to, false, false)),
        (
            "jump to the next pc",
            enc::jump(TRUE, CODE + 7, false, false),
        ),
        ("if eq jump, not taken", enc::jump(eq, to, false, false)),
        ("if ne jump, taken", enc::jump(ne, to, false, false)),
        ("call", enc::jump(TRUE, to, true, false)),
        ("jump (m,i)", enc::jump_i(TRUE, 8, 8, false, false)),
        ("jump pc6", enc::jump_pc6(TRUE, 5, false, false)),
        ("rts", enc::rts(TRUE, false, false)),
        ("ret wide", enc::ret_wide(TRUE, false, false)),
        ("nested do", enc::do_count(2, 3)),
        ("do until", enc::do_until(31, 3)),
    ]
}

#[test]
fn a_terminator_that_is_the_last_instruction_of_a_loop_body_takes_the_back_edge() {
    let mut total = Tally::default();
    for (name, last) in terminators_at_a_loop_end() {
        for az in [false, true] {
            let (program, _) = loop_ending_in(&last, 3);
            let mut rig = Rig::new(&program, |c| {
                c.flags.az = az;
                (c.i[8], c.m[8]) = (CODE + 12, 0);
                for k in 0..3 {
                    c.push_return_address(CODE + 8 + k);
                }
            });
            let terminator = |u: &&Uop| KINDS.iter().any(|k| k.0 == u.h) && u.h != SYS_LOAD.0;
            let fin = rig
                .uops
                .iter()
                .filter(terminator)
                .any(|u| u.flags & UF_FIN != 0);
            assert!(!fin, "{name}: the translator gave a terminator UF_FIN");
            rig.run(40)
                .unwrap_or_else(|e| panic!("{name} az={az}: {e}"));
            assert!(rig.tally.blocks >= 1, "{name}: {:?}", rig.tally);
            total.merge(&rig.tally);
        }
    }
    for k in [JUMP, JUMP_I, RET, DO_COUNT] {
        assert!(total.calls(k) > 0, "{k:?} never ran: {total:?}");
    }
}

#[test]
fn a_jump_at_the_loop_end_jumps_back_until_the_last_pass_and_then_leaves() {
    let to = CODE + 12;
    let (program, after) = loop_ending_in(&enc::jump(TRUE, to, false, false), 3);
    let mut rig = Rig::new(&program, |_| {});
    rig.run_to(to, 40).unwrap();
    assert_eq!(
        rig.b.steps,
        1 + 3 * 2,
        "DO, then three passes of NOP and JUMP"
    );
    assert_eq!(rig.tally.calls(JUMP), 3);
    assert_eq!(rig.tally.exit(Exit::Jump), 3, "two back-edges and the exit");
    assert!(after < to && rig.b.loops.is_empty());
}

/// A mutant `JUMP` that never asks `finish_step` what comes next: it jumps and stops there, as a
/// handler would that left the loop back-edge to the dispatch loop's `UF_FIN` (which a `Jump`
/// exit never runs).
unsafe extern "C" fn bare_jump(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    // SAFETY: called by `run_uops` with the live context and a uop of the block, as the real ones.
    let (ctx, u) = unsafe { (&mut *ctx, &*u) };
    ctx.next_pc = u.a;
    Exit::Jump as u32
}

#[test]
fn a_terminator_that_leaves_its_loop_back_edge_to_nobody_is_caught_at_a_loop_end() {
    let to = CODE + 12;
    let (program, _) = loop_ending_in(&enc::jump(TRUE, to, false, false), 3);
    let mut honest = Rig::new(&program, |_| {});
    honest
        .run_to(to, 40)
        .expect("the real handler takes the back-edge itself");
    assert_eq!(honest.b.steps, 1 + 3 * 2, "three passes");
    let mut mutant = Rig::new(&program, |_| {});
    mutant.replace_handler(JUMP, bare_jump);
    assert!(
        mutant.run(40).is_err(),
        "a jump that skips the back-edge left the loop after one pass unnoticed"
    );
}

#[test]
fn a_forced_fin_on_a_delayed_branch_resolves_it_one_slot_early_and_is_caught() {
    let to = CODE + 12;
    let mut p = enc::jump(TRUE, to, false, true);
    p.extend([NOP, NOP]);
    p.extend([NOP; 12]);
    let fins = |rig: &Rig| {
        rig.uops
            .iter()
            .filter(|u| u.h == JUMP.0 && u.flags & UF_FIN != 0)
            .count()
    };
    let mut honest = Rig::new(&p, |_| {});
    assert_eq!(
        fins(&honest),
        0,
        "the translator gives the branch no UF_FIN"
    );
    honest.run_to(to, 10).expect("the two slots resolve it");
    assert_eq!(honest.b.steps, 3, "the branch and its two delay slots");
    assert_eq!(honest.tally.fin_jumps, 1, "the second slot is the jump");
    let mut forced = Rig::new(&p, |_| {});
    for u in forced.uops.iter_mut().filter(|u| u.h == JUMP.0) {
        u.flags |= UF_FIN;
    }
    assert_eq!(fins(&forced), 2, "one JUMP uop per PEy mode");
    assert!(
        forced.run(10).is_err(),
        "the countdown ran one step too many and nobody noticed"
    );
}

#[test]
fn calls_and_returns_nest_with_and_without_delay_slots() {
    let (sub1, sub2, end) = (CODE + 30, CODE + 50, CODE + 70);
    let mut a = Asm::default();
    a.put(&enc::jump(TRUE, sub1, true, false));
    a.put(&enc::jump(TRUE, sub1, true, true)); // delayed call: the slots run first
    a.nops(2);
    a.put(&enc::jump(TRUE, end, false, false));
    a.pad_to(sub1);
    a.put(&[NOP]);
    a.put(&enc::jump(TRUE, sub2, true, false));
    a.put(&[NOP]);
    a.put(&enc::rts(TRUE, false, false));
    a.pad_to(sub2);
    a.put(&[NOP]);
    a.put(&enc::rts(TRUE, true, false)); // delayed return
    a.nops(2);
    a.pad_to(end + 4);
    let mut rig = Rig::new(&a.0, |_| {});
    rig.run_to(end, 100).unwrap();
    assert_eq!(rig.tally.calls(RET), 4, "two calls of each subroutine");
    assert_eq!(
        rig.tally.calls(JUMP),
        2 + 2 + 1,
        "two calls, two nested, the exit"
    );
    assert_eq!(rig.tally.stepped, 0, "{:?}", rig.tally);
    assert!(rig.b.call_stack.is_empty());
}

#[test]
fn conditional_branches_and_indirect_jumps_follow_the_flags_and_the_registers() {
    let mut a = Asm::default();
    a.put(&enc::jump(0, CODE + 20, false, false)); // IF EQ JUMP
    a.nops(2);
    a.put(&enc::jump_i(TRUE, 8, 9, false, true)); // JUMP (M9, I8) (DB) from pc 5
    a.nops(2);
    a.pad_to(CODE + 20);
    a.put(&enc::jump(1, CODE + 40, true, true)); // IF NE CALL (DB)
    a.nops(2);
    a.put(&enc::jump_i(TRUE, 8, 9, false, false));
    a.pad_to(CODE + 40);
    a.put(&enc::rts(TRUE, false, false));
    a.nops(20);
    for az in [false, true] {
        let mut rig = Rig::new(&a.0, |c| {
            c.flags.az = az;
            (c.i[8], c.m[9]) = (CODE + 30, 2);
            c.push_return_address(CODE + 60);
        });
        rig.run(30).unwrap_or_else(|e| panic!("az={az}: {e}"));
        assert!(
            rig.tally.calls(JUMP) > 0 && rig.tally.calls(JUMP_I) > 0,
            "az={az}: {:?}",
            rig.tally
        );
    }
}

/// `DO (PC, end) UNTIL LCE, LCNTR = ureg` entered after `lead` NOPs of the same block, on a core
/// that already ran `entry` steps, so the pc and the step count the handler sees are not the ones
/// of the block entry. Returns the rig after one block.
fn count_from_register_after(ureg: u8, lead: usize, entry: u64) -> Rig {
    let mut a = Asm::default();
    a.nops(lead);
    a.put(&enc::do_reg(ureg, 5));
    a.nops(8);
    let mut rig = Rig::new(&a.0, |c| {
        c.steps = entry;
        // a running timer whose count falls one per step, so TCOUNT depends on the step count
        c.write_ureg(0x6F, 1000);
        c.write_ureg(0x6E, 5000);
        c.write_ureg(0x74, crate::core::timer::TIMEN);
        c.irq.due = u64::MAX;
    });
    rig.advance()
        .unwrap_or_else(|e| panic!("ureg {ureg:#x}: {e}"));
    rig
}

#[test]
fn a_loop_count_read_from_a_register_that_sees_the_pc_or_the_steps_is_the_interpreters_value() {
    // (register, steps before the block, the count the interpreter reads at the DO): the DO is
    // the sixth instruction, so the interpreter's pc is CODE + 5 and it has run 5 steps more
    let lead = 5;
    let cases = [
        (0x60u8, 10u64, u64::from(CODE) + 5), // FADDR
        (0x61, 10, u64::from(CODE) + 5),      // DADDR
        (0x63, 10, u64::from(CODE) + 5),      // PC
        (0x69, 10, 10 + 5),                   // EMUCLK: the cycle clock, low word
        (0x6A, (1 << 32) - 2, 1),             // EMUCLK2: carries into the high word in the block
        (0x6F, 10, 1000 - 3),                 // TCOUNT: running since the cycle 12, read at 15
    ];
    for (ureg, entry, want) in cases {
        let rig = count_from_register_after(ureg, lead, entry);
        let frame = rig
            .a
            .loops
            .last()
            .copied()
            .expect("the oracle pushed a loop");
        assert_eq!(u64::from(frame.remaining), want, "ureg {ureg:#x}: oracle");
        assert_eq!(
            rig.b.loops.last().map(|f| f.remaining),
            Some(frame.remaining),
            "ureg {ureg:#x}: engine"
        );
        assert_eq!(
            (rig.tally.calls(DO_COUNT), rig.tally.exit(Exit::Fault)),
            (1, 0),
            "ureg {ureg:#x}: the handler ran and did not bail: {:?}",
            rig.tally
        );
    }
}

#[test]
fn a_mutated_uop_stream_is_caught_by_the_chain_comparison() {
    let (sub, end) = (CODE + 10, CODE + 20);
    let mut a = Asm::default();
    a.put(&enc::jump(TRUE, sub, true, false));
    a.put(&enc::jump(TRUE, end, false, false));
    a.pad_to(sub);
    a.put(&enc::rts(TRUE, false, false));
    a.pad_to(end + 4);
    let mutations: [fn(&mut Uop); 2] = [|u| u.b ^= 1 << 8, |u| u.a += 1];
    for mutate in mutations {
        let mut rig = Rig::new(&a.0, |_| {});
        rig.uops
            .iter_mut()
            .filter(|u| u.h >= 0x400)
            .for_each(mutate);
        assert!(
            rig.run(20).is_err(),
            "a wrong operand reached the end unnoticed"
        );
    }
}
