//! OWNER: W1-FLOW (tests). `lower_flow` ender by ender, with the context a trace builder would
//! hand it: which uops and exits each branch, call, return and register jump produces, the
//! guard polarity, the caps and the fall-backs to `GENERIC_END`.

use super::*;
use crate::machine::sb::decode::decode;
use crate::machine::sb::trace::shape::{b, bcc, bl, BX_LR, MOV_PC_LR};
use crate::machine::sb::trace::testexec::suite;

const PC: u32 = 0xC000_0100;
const ENTRY: u32 = 0xC000_0000;
const BX_R3: u32 = 0xE12F_FF13;
const MOV_PC_R4: u32 = 0xE1A0_F004;

/// The context of one `lower_flow` call.
struct Case {
    done: u16,
    hint: Option<bool>,
    follow: bool,
    opts: BuildOpts,
    depth: u8,
    rets: Vec<u32>,
    uops: Vec<Uop>,
    exits: Vec<Exit>,
}

fn case() -> Case {
    Case {
        done: 3,
        hint: None,
        follow: true,
        opts: BuildOpts::default(),
        depth: 0,
        rets: Vec::new(),
        uops: Vec::new(),
        exits: Vec::new(),
    }
}

struct Out {
    flow: Flow,
    uops: Vec<Uop>,
    exits: Vec<Exit>,
    depth: u8,
    rets: Vec<u32>,
}

fn lower(word: u32, c: Case) -> Out {
    let Case {
        done,
        hint,
        follow,
        opts,
        mut depth,
        mut rets,
        mut uops,
        mut exits,
    } = c;
    let can = move |_: u32| follow;
    let flow = lower_flow(&mut FlowCtx {
        insn: decode(PC, word),
        uops: &mut uops,
        exits: &mut exits,
        done,
        taken_hint: hint,
        can_follow: &can,
        depth: &mut depth,
        rets: &mut rets,
        entry_pc: ENTRY,
        opts: &opts,
    });
    Out {
        flow,
        uops,
        exits,
        depth,
        rets,
    }
}

fn with_guards(n: usize) -> Vec<Uop> {
    (0..n)
        .map(|i| {
            Uop::new(
                if i % 2 == 0 {
                    k::GUARD
                } else {
                    k::GUARD_REG_EQ
                },
                0,
            )
        })
        .collect()
}

fn filler(n: usize) -> Vec<Exit> {
    vec![Exit::new(ExitKind::Stale, 1, 0, 0); n]
}

/// The exit `o.exits[i]` as `(kind, reg, done, last_pc, target)`.
fn exit(o: &Out, i: usize) -> (ExitKind, u8, u16, u32, u32) {
    let e = &o.exits[i];
    (e.kind, e.reg, e.done, e.last_pc, e.target)
}

/// The uop `o.uops[i]` as `(kind, n, exit index, cond, pc)`.
fn uop(o: &Out, i: usize) -> (u8, u8, usize, u8, u32) {
    let u = &o.uops[i];
    (u.kind, u.n, u.exit_idx(), u.cond, u.pc)
}

fn assert_generic_end(o: &Out, word: u32, done: u16) {
    assert_eq!(o.flow, Flow::End, "{word:#x}");
    let u = o.uops.last().unwrap();
    assert_eq!(
        (u.kind, u.imm, u.pc, u.n),
        (k::GENERIC_END, word, PC, 1),
        "{word:#x}"
    );
    assert_eq!(
        exit(o, u.exit_idx()),
        (ExitKind::Dynamic, 0, done, PC, 0),
        "{word:#x}"
    );
}

suite! {
    #[test]
    fn an_unconditional_branch_is_followed_without_a_uop_and_ends_in_an_exit_when_it_cannot_be() {
        let word = b(PC, PC + 0x40);
        let o = lower(word, case());
        assert_eq!(o.flow, Flow::Follow(PC + 0x40));
        assert!(o.uops.is_empty() && o.exits.is_empty());
        let o = lower(word, Case { follow: false, exits: filler(2), done: 9, ..case() });
        assert_eq!(o.flow, Flow::End);
        assert_eq!(uop(&o, 0), (k::EXIT, 1, 2, 0, PC));
        assert_eq!(exit(&o, 2), (ExitKind::Direct, 0, 10, PC, PC + 0x40));
        // a branch back to the entry closes the loop whether or not the target could be followed
        for follow in [true, false] {
            let o = lower(b(PC, ENTRY), Case { follow, ..case() });
            assert_eq!(o.flow, Flow::End);
            assert_eq!(uop(&o, 0), (k::LOOP_BACK, 1, 0, 0, PC));
            assert_eq!(exit(&o, 0), (ExitKind::LoopBack, 0, 4, PC, ENTRY));
        }
    }

    #[test]
    fn a_conditional_branch_guards_the_side_it_does_not_follow_for_every_condition() {
        for cond in 0..14u32 {
            let c = cond as u8;
            // forward: predicted not taken, the guard fires when the branch is taken
            let o = lower(bcc(cond, PC, PC + 0x40), Case { exits: filler(1), ..case() });
            assert_eq!(o.flow, Flow::Follow(PC + 4), "cond {cond}");
            assert_eq!((o.uops.len(), uop(&o, 0)), (1, (k::GUARD, 1, 1, c, PC)), "cond {cond}");
            assert_eq!(exit(&o, 1), (ExitKind::Direct, 0, 4, PC, PC + 0x40), "cond {cond}");
            // backward: predicted taken, the guard fires when the branch falls through
            let o = lower(bcc(cond, PC, PC - 0x40), case());
            assert_eq!(o.flow, Flow::Follow(PC - 0x40), "cond {cond}");
            assert_eq!(uop(&o, 0), (k::GUARD, 1, 0, c ^ 1, PC), "cond {cond}");
            assert_eq!(exit(&o, 0), (ExitKind::Direct, 0, 4, PC, PC + 4), "cond {cond}");
            // the hint overrides the static rule, both ways
            let o = lower(bcc(cond, PC, PC + 0x40), Case { hint: Some(true), ..case() });
            assert_eq!((o.flow, uop(&o, 0).3), (Flow::Follow(PC + 0x40), c ^ 1), "cond {cond}");
            let o = lower(bcc(cond, PC, PC - 0x40), Case { hint: Some(false), ..case() });
            assert_eq!((o.flow, uop(&o, 0).3), (Flow::Follow(PC + 4), c), "cond {cond}");
        }
    }

    #[test]
    fn a_conditional_branch_whose_followed_side_is_refused_or_is_the_entry_ends_the_trace() {
        // refused: the guard, then an exit towards the predicted side
        let o = lower(bcc(1, PC, PC - 0x40), Case { follow: false, done: 5, ..case() });
        assert_eq!(o.flow, Flow::End);
        assert_eq!((uop(&o, 0), uop(&o, 1)), ((k::GUARD, 1, 0, 0, PC), (k::EXIT, 0, 1, 0, PC)));
        assert_eq!(exit(&o, 0), (ExitKind::Direct, 0, 6, PC, PC + 4));
        assert_eq!(exit(&o, 1), (ExitKind::Direct, 0, 6, PC, PC - 0x40));
        // the entry: a loop back, the instruction counted by the guard
        let o = lower(bcc(1, PC, ENTRY), case());
        assert_eq!(o.flow, Flow::End);
        assert_eq!((uop(&o, 0), uop(&o, 1)), ((k::GUARD, 1, 0, 0, PC), (k::LOOP_BACK, 0, 1, 0, PC)));
        assert_eq!(exit(&o, 1), (ExitKind::LoopBack, 0, 4, PC, ENTRY));
        // a fall-through that cannot be followed
        let o = lower(bcc(1, PC, PC + 0x40), Case { follow: false, ..case() });
        assert_eq!(exit(&o, 1), (ExitKind::Direct, 0, 4, PC, PC + 4));
    }

    #[test]
    fn guards_are_capped_by_the_options_and_the_exit_table_by_its_size() {
        let word = bcc(1, PC, PC + 0x40);
        let capped = |guards: usize, cap: usize| {
            let opts = BuildOpts { max_guards: cap, ..BuildOpts::default() };
            lower(word, Case { uops: with_guards(guards), opts, ..case() })
        };
        assert_generic_end(&capped(2, 2), word, 4);
        assert_generic_end(&capped(0, 0), word, 4);
        assert_eq!(capped(1, 2).flow, Flow::Follow(PC + 4), "one below the cap");
        assert_eq!(capped(2, 3).uops.last().unwrap().kind, k::GUARD);
        // a branch that needs no guard is not held back by the cap
        let o = lower(b(PC, PC + 8), Case { uops: with_guards(9), ..case() });
        assert_eq!(o.flow, Flow::Follow(PC + 8));
        // two exits must fit: the exit table
        let o = lower(word, Case { exits: filler(MAX_EXITS - 1), ..case() });
        assert_generic_end(&o, word, 4);
        assert_eq!(o.exits.len(), MAX_EXITS);
        let o = lower(b(PC, PC + 8), Case { exits: filler(MAX_EXITS - 1), ..case() });
        assert_generic_end(&o, b(PC, PC + 8), 4);
        let o = lower(word, Case { exits: filler(MAX_EXITS - 2), ..case() });
        assert_eq!((o.flow, o.exits.len()), (Flow::Follow(PC + 4), MAX_EXITS - 1));
    }

    #[test]
    fn a_call_links_and_is_inlined_to_the_depth_limit() {
        let word = bl(PC, PC + 0x40);
        let o = lower(word, Case { rets: vec![0x111], depth: 1, ..case() });
        assert_eq!(o.flow, Flow::Follow(PC + 0x40));
        let u = &o.uops[0];
        assert_eq!((o.uops.len(), u.kind, u.rd, u.imm, u.pc, u.n), (1, k::MOV_IMM, 14, PC + 4, PC, 1));
        assert_eq!((o.depth, o.rets.clone(), o.exits.len()), (2, vec![0x111, PC + 4], 0));
        // at the limit, with inlining off, or with a callee that cannot be followed: link and exit
        let at_limit = Case { depth: 2, ..case() };
        let off = Case { opts: BuildOpts { inline_depth: 0, ..BuildOpts::default() }, ..case() };
        let refused = Case { follow: false, ..case() };
        for c in [at_limit, off, refused] {
            let (depth, rets) = (c.depth, c.rets.clone());
            let o = lower(word, Case { done: 6, ..c });
            assert_eq!(o.flow, Flow::End);
            assert_eq!((o.uops[0].kind, o.uops[0].imm), (k::MOV_IMM, PC + 4));
            assert_eq!(uop(&o, 1), (k::EXIT, 0, 0, 0, PC));
            assert_eq!(exit(&o, 0), (ExitKind::Direct, 0, 7, PC, PC + 0x40));
            assert_eq!((o.depth, o.rets), (depth, rets), "nothing was inlined");
        }
        // a call to the entry links, then loops back; it is never inlined
        let o = lower(bl(PC, ENTRY), case());
        assert_eq!((o.uops[0].kind, uop(&o, 1)), (k::MOV_IMM, (k::LOOP_BACK, 0, 0, 0, PC)));
        assert_eq!((exit(&o, 0), o.depth, o.rets.len()), ((ExitKind::LoopBack, 0, 4, PC, ENTRY), 0, 0));
        // a conditional call stays with the interpreter, link included
        assert_generic_end(&lower(0x0B00_0010, case()), 0x0B00_0010, 4);
    }

    #[test]
    fn bx_and_mov_pc_end_in_a_register_exit_with_their_own_interworking() {
        for (word, kind, exit_kind, rm) in [
            (BX_R3, k::EXIT_REG, ExitKind::Indirect, 3u8),
            (BX_LR, k::EXIT_REG, ExitKind::Indirect, 14),
            (MOV_PC_R4, k::EXIT_REG_ARM, ExitKind::IndirectArm, 4),
            (MOV_PC_LR, k::EXIT_REG_ARM, ExitKind::IndirectArm, 14),
        ] {
            let o = lower(word, Case { exits: filler(1), ..case() });
            assert_eq!(o.flow, Flow::End, "{word:#x}");
            assert_eq!(uop(&o, 0), (kind, 1, 1, 0, PC), "{word:#x}");
            assert_eq!(o.uops[0].rn, rm, "{word:#x}");
            assert_eq!(exit(&o, 1), (exit_kind, rm, 4, PC, 0), "{word:#x}");
        }
    }

    #[test]
    fn a_return_to_an_inlined_call_is_guarded_and_followed_for_both_forms() {
        for (word, exit_kind) in [(BX_LR, ExitKind::Indirect), (MOV_PC_LR, ExitKind::IndirectArm)] {
            let live = Case { depth: 2, rets: vec![0x50, 0x1234], ..case() };
            let o = lower(word, live);
            assert_eq!(o.flow, Flow::Follow(0x1234), "{word:#x}");
            let u = &o.uops[0];
            assert_eq!((u.kind, u.rn, u.imm, u.n, u.pc), (k::GUARD_REG_EQ, 14, 0x1234, 1, PC));
            assert_eq!(exit(&o, u.exit_idx()), (exit_kind, 14, 4, PC, 0), "{word:#x}");
            assert_eq!((o.depth, o.rets), (1, vec![0x50]), "the return pops its call");
            // the return address is the entry, or cannot be followed: guard, then a second exit
            let o = lower(word, Case { depth: 1, rets: vec![ENTRY], ..case() });
            assert_eq!((o.uops[1].kind, o.exits.len()), (k::LOOP_BACK, 2), "{word:#x}");
            assert_eq!(exit(&o, 1), (ExitKind::LoopBack, 0, 4, PC, ENTRY), "{word:#x}");
            let o = lower(word, Case { depth: 1, rets: vec![0x1234], follow: false, ..case() });
            assert_eq!((o.uops[0].kind, uop(&o, 1)), (k::GUARD_REG_EQ, (k::EXIT, 0, 1, 0, PC)));
            assert_eq!(exit(&o, 1), (ExitKind::Direct, 0, 4, PC, 0x1234), "{word:#x}");
        }
        // nothing to return to, another register, or no room for a guard: a plain register exit
        let nothing = Case { depth: 1, ..case() };
        let other = Case { depth: 1, rets: vec![0x77], ..case() };
        let full = Case {
            depth: 1,
            rets: vec![0x77],
            opts: BuildOpts { max_guards: 0, ..BuildOpts::default() },
            ..case()
        };
        for (word, c, depth, rets) in [(BX_LR, nothing, 1, 0), (BX_R3, other, 1, 1), (BX_LR, full, 1, 1)] {
            let o = lower(word, c);
            assert_eq!((o.uops[0].kind, o.depth, o.rets.len()), (k::EXIT_REG, depth, rets), "{word:#x}");
        }
    }

    #[test]
    fn a_conditional_register_jump_guards_through_the_register_and_never_inlines_a_return() {
        for (word, exit_kind, rm) in [
            (0x012F_FF13, ExitKind::Indirect, 3u8),     // bxeq r3
            (0x112F_FF1E, ExitKind::Indirect, 14),      // bxne lr
            (0x01A0_F004, ExitKind::IndirectArm, 4),    // moveq pc,r4
            (0x11A0_F00E, ExitKind::IndirectArm, 14),   // movne pc,lr
        ] {
            let cond = (word >> 28) as u8;
            let o = lower(word, Case { depth: 1, rets: vec![0x1234], ..case() });
            assert_eq!(o.flow, Flow::Follow(PC + 4), "{word:#x}");
            assert_eq!(uop(&o, 0), (k::GUARD, 1, 0, cond, PC), "{word:#x}");
            assert_eq!(exit(&o, 0), (exit_kind, rm, 4, PC, 0), "{word:#x}");
            assert_eq!((o.depth, o.rets), (1, vec![0x1234]), "{word:#x}");
            let opts = BuildOpts { max_guards: 0, ..BuildOpts::default() };
            assert_generic_end(&lower(word, Case { opts, ..case() }), word, 4);
        }
        // the fall-through cannot be followed
        let o = lower(0x012F_FF13, Case { follow: false, ..case() });
        assert_eq!((o.uops[1].kind, o.uops[1].n, exit(&o, 1).4), (k::EXIT, 0, PC + 4));
    }

    #[test]
    fn everything_else_goes_to_the_interpreter_with_the_static_done_and_last_pc() {
        for word in [
            0xE591_F000, // ldr pc,[r1]
            0xE8BD_8000, // ldmfd sp!,{pc}
            0xEF00_0000, // swi
            0xE12F_FF33, // blx r3
            0xFA00_0000, // blx label
            0xE12F_FF1F, // bx pc
            0xE1A0_F00F, // mov pc,pc
            0xE1B0_F00E, // movs pc,lr
            0xE1A0_F11E, // mov pc,lr,lsl r1
            0xE1A0_F10E, // mov pc,lr,lsl #2
            0xE28E_F004, // add pc,lr,#4
            0xE10F_3000, // mrs r3,cpsr
        ] {
            for done in [0u16, 7, 1000] {
                let o = lower(word, Case { done, exits: filler(3), ..case() });
                assert_generic_end(&o, word, done + 1);
                assert_eq!((o.uops.len(), o.exits.len(), o.uops[0].exit_idx()), (1, 4, 3), "{word:#x}");
            }
        }
    }
}
