//! Tests of the flow family inside the translator: the shape of the blocks built around it.
//!
//! OWNER: W2-FLOW. The translator (`translate.rs`) and the family share one contract (design P1):
//!
//! * a terminator (jump, call, return, `DO`, `IDLE`) runs `finish_step` in its handler, so it never
//!   gets `UF_FIN`, also when it is the last instruction of a static loop body (the inherited P1
//!   item of W1-LOWER: with a flag the handler and the dispatch loop would take the back-edge
//!   twice);
//! * the two delay slots of a delayed branch and a plain instruction that ends a loop body do get
//!   `UF_FIN`, a system-register load among them (it is a `Plain` instruction for the translator);
//! * a block that ends in a terminator or a flagged uop tells the entry guard so (`FP_LAST_SEQ`).
//!
//! The executor side of the same contract (the handlers against `Core::step`, with negative
//! controls that force the flag) is `exec/h_flow/diff_chain.rs`.
use crate::core::support::{core_with, CODE};
use crate::core::uop::config::UopConfig;
use crate::core::uop::exec::h_flow::enc::{self, Asm, NOP, TRUE};
use crate::core::uop::exec::h_flow::harness::flow_only;
use crate::core::uop::exec::h_flow::{JUMP, JUMP_I, KINDS, RET, SYS_LOAD};
use crate::core::uop::lower::builder::FP_LAST_SEQ;
use crate::core::uop::lower::cut::Flow;
use crate::core::uop::lower::support::{build, lockstep, Built, FLOW_CASES, LOAD_SYS};
use crate::core::uop::types::{Block, GuardClass, Uop, UopKind, UF_FIN, UF_LAST};

/// `DO (PC, CODE + 4) UNTIL LCE, 3`, a NOP, `last` at `CODE + 4` (the loop end), then NOPs for
/// its delay slots and the code after the loop.
fn loop_ending_in(last: &[u16]) -> Vec<u16> {
    let mut p = enc::do_count(3, 4);
    p.push(NOP);
    p.extend_from_slice(last);
    p.extend([NOP; 8]);
    p
}

/// The block of mode `simd` that holds the instruction at `pc`, with its uops.
fn holding(built: &Built, simd: bool, pc: u32) -> Option<(&Block, &[Uop])> {
    built
        .mode(simd)
        .into_iter()
        .find(|(_, uops)| uops.iter().any(|u| u.pc == pc && u.steps > 0))
}

fn built_with_flow(program: &[u16]) -> Built {
    build(program, &UopConfig::default(), None, flow_only).1
}

#[test]
fn a_terminator_gets_no_fin_also_when_it_is_the_last_instruction_of_a_static_loop_body() {
    let at = CODE + 4;
    let (mut taken, mut declined) = (Vec::new(), Vec::new());
    for (name, parcels, flow) in FLOW_CASES {
        let Flow::End { slots } = flow else { continue };
        let built = built_with_flow(&loop_ending_in(parcels));
        for simd in [false, true] {
            let (blk, uops) =
                holding(&built, simd, at).unwrap_or_else(|| panic!("{name}: no block"));
            let i = uops.iter().position(|u| u.pc == at && u.steps > 0).unwrap();
            let u = uops[i];
            assert_eq!(
                (uops[0].pc, uops[0].flags & UF_FIN),
                (CODE + 3, 0),
                "{name}: the body starts a block, the NOP before the end is not a loop end"
            );
            assert_eq!(u.flags & UF_FIN, 0, "{name}: a terminator never has UF_FIN");
            if !KINDS.iter().any(|k| k.0 == u.h) {
                // left to the interpreter, which finishes its own step
                assert_eq!(u.h, UopKind::INTERP.0, "{name}");
                if !simd {
                    declined.push(*name);
                }
                continue;
            }
            assert_ne!(blk.footprint.writes & FP_LAST_SEQ, 0, "{name}: sequenced");
            if *slots > 0 {
                let after: Vec<u8> = uops[i + 1..]
                    .iter()
                    .map(|u| u.flags & (UF_FIN | UF_LAST))
                    .collect();
                assert_eq!(
                    after,
                    [UF_FIN, UF_FIN | UF_LAST],
                    "{name}: the slots resolve it"
                );
                assert_eq!(blk.guard, GuardClass::Delayed, "{name}");
            } else {
                assert_eq!(i + 1, uops.len(), "{name}: ends the block");
                assert_eq!(u.flags & (UF_FIN | UF_LAST), UF_LAST, "{name}");
                assert_eq!(blk.guard, GuardClass::Loop, "{name}: the loop end was seen");
            }
            if !simd {
                taken.push(*name);
            }
        }
    }
    assert_eq!(taken.len(), 18, "the forms the family lowers: {taken:?}");
    assert_eq!(
        declined.len(),
        7,
        "stack ops and CJUMP stay Interp: {declined:?}"
    );
}

#[test]
fn a_jump_with_a_loop_modifier_at_a_loop_end_is_interp_and_sequenced_without_fin() {
    for (name, w1) in [("JUMP (LA)", 0x063E | 0x0040), ("JUMP (CI)", 0x063E)] {
        let ci = if name.ends_with("(CI)") { 0x0100 } else { 0 };
        let built = built_with_flow(&loop_ending_in(&[w1, ci, 0x0010]));
        let (blk, uops) = holding(&built, false, CODE + 4).unwrap();
        let u = uops.iter().find(|u| u.pc == CODE + 4).unwrap();
        assert_eq!((u.h, u.flags & UF_FIN), (UopKind::INTERP.0, 0), "{name}");
        assert_ne!(
            blk.footprint.writes & FP_LAST_SEQ,
            0,
            "{name}: Interp sequences"
        );
    }
}

#[test]
fn the_delay_slots_carry_fin_and_the_branch_does_not_for_every_kind_of_delayed_branch() {
    for (name, kind) in [
        ("jump db", JUMP),
        ("jump (m,i) db", JUMP_I),
        ("call db", JUMP),
        ("rts db", RET),
        ("rts wide db", RET),
    ] {
        let (_, parcels, _) = FLOW_CASES.iter().find(|(n, ..)| *n == name).unwrap();
        let built = built_with_flow(&[&parcels[..], &[NOP; 6]].concat());
        for simd in [false, true] {
            let (blk, uops) = built.at(CODE, simd).unwrap();
            let flags: Vec<(u16, u8)> = uops
                .iter()
                .map(|u| (u.h, u.flags & (UF_FIN | UF_LAST)))
                .collect();
            assert_eq!(
                flags,
                [
                    (kind.0, 0),
                    (UopKind::NOP.0, UF_FIN),
                    (UopKind::NOP.0, UF_FIN | UF_LAST)
                ],
                "{name}"
            );
            assert_eq!((blk.guard, blk.steps), (GuardClass::Delayed, 3), "{name}");
            assert_ne!(blk.footprint.writes & FP_LAST_SEQ, 0, "{name}");
        }
    }
}

#[test]
fn a_system_register_load_gets_fin_as_a_loop_end_or_a_slot_and_never_otherwise() {
    let load = LOAD_SYS.to_vec();
    // a plain load ends its block and is not flagged: nothing finishes a step there
    let built = built_with_flow(&[&[NOP, NOP][..], &load, &[NOP; 4]].concat());
    let (blk, uops) = built.at(CODE, false).unwrap();
    let flags: Vec<(u16, u8)> = uops
        .iter()
        .map(|u| (u.h, u.flags & (UF_FIN | UF_LAST)))
        .collect();
    assert_eq!(
        flags,
        [
            (UopKind::NOP.0, 0),
            (UopKind::NOP.0, 0),
            (SYS_LOAD.0, UF_LAST)
        ]
    );
    assert_eq!(
        (blk.end_pc, blk.footprint.writes & FP_LAST_SEQ),
        (CODE + 4, 0)
    );
    // the last instruction of a loop body is flagged, so the dispatch loop takes the back-edge
    let built = built_with_flow(&loop_ending_in(&load));
    let (blk, uops) = holding(&built, false, CODE + 4).unwrap();
    let last = uops.last().unwrap();
    assert_eq!(
        (last.h, last.flags & (UF_FIN | UF_LAST)),
        (SYS_LOAD.0, UF_FIN | UF_LAST)
    );
    assert_eq!(blk.guard, GuardClass::Loop);
    assert_ne!(blk.footprint.writes & FP_LAST_SEQ, 0);
    // the second delay slot is flagged too; a load in the first slot keeps the group away
    let second = [
        &enc::jump(TRUE, CODE + 20, false, true)[..],
        &[NOP],
        &load,
        &[NOP; 4],
    ]
    .concat();
    let built = built_with_flow(&second);
    let (_, uops) = built.at(CODE, false).unwrap();
    let flags: Vec<(u16, u8)> = uops
        .iter()
        .map(|u| (u.h, u.flags & (UF_FIN | UF_LAST)))
        .collect();
    assert_eq!(
        flags,
        [
            (JUMP.0, 0),
            (UopKind::NOP.0, UF_FIN),
            (SYS_LOAD.0, UF_FIN | UF_LAST)
        ]
    );
    let first = [
        &enc::jump(TRUE, CODE + 20, false, true)[..],
        &load,
        &[NOP; 5],
    ]
    .concat();
    let built = built_with_flow(&first);
    assert!(
        holding(&built, false, CODE).is_none(),
        "the branch whose first slot loads a system register is not translated"
    );
}

#[test]
fn nested_loops_ended_by_terminators_are_cut_into_blocks_the_interpreter_walks_exactly() {
    let mut a = Asm::default();
    a.put(&enc::do_count(3, 12)); // outer, ends at CODE + 12
    a.put(&[NOP]);
    a.put(&enc::do_count(2, 4)); // inner at CODE + 4, ends at CODE + 8
    a.put(&[NOP]);
    a.put(&enc::jump(TRUE, CODE + 11, false, false)); // inner end: a jump to the next pc
    a.put(&[NOP]);
    a.put(&enc::jump(TRUE, CODE + 15, false, false)); // outer end
    a.nops(8);
    let (mut b, built) = build(&a.0, &UopConfig::default(), None, flow_only);
    let mut oracle = core_with(&a.0);
    (b.irq.due, oracle.irq.due) = (u64::MAX, u64::MAX);
    let seen = lockstep(&mut oracle, &mut b, &built, None, 200).unwrap();
    assert_eq!(
        seen.steps,
        1 + 3 * (1 + 1 + 2 * 2 + 1 + 1) + 8,
        "every step of both loops and the code after them: {seen:?}"
    );
    assert!(seen.loop_backs > 0 && seen.fell_through > 0, "{seen:?}");
    assert!(b.loops.is_empty(), "both loops ran to their last pass");
}
