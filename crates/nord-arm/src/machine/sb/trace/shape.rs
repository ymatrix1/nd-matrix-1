//! OWNER: W1-FLOW (tests). Shape tests of `trace::build`: which instructions are followed, what
//! the guards and exits of a trace say (`done`, `last_pc`, targets, polarity), where traces stop
//! and why, and that the snapshot (`words`, pages) equals memory.
//!
//! The tests look at the control uops and at the exits, never at the uops of ordinary
//! instructions, so they hold with the skeleton's `GENERIC` lowering and with the real lowerers
//! of W1-DP and W1-MEM alike (every ordinary instruction used here lowers to one uop).

use super::super::uop::{k, Exit, ExitKind, Sb, Uop, MAX_EXITS, MAX_UOPS};
use super::testexec::suite;
use super::{build, build_flipped, BuildOpts};
use crate::mem::Mem;

pub(super) const C: u32 = 0xC000_0000;
pub(in crate::machine::sb) const BX_LR: u32 = 0xE12F_FF1E;
pub(in crate::machine::sb) const MOV_PC_LR: u32 = 0xE1A0_F00E;

pub(in crate::machine::sb) fn mov(rd: u32, imm: u32) -> u32 {
    0xE3A0_0000 | rd << 12 | imm
}

pub(in crate::machine::sb) fn add(rd: u32, imm: u32) -> u32 {
    0xE280_0000 | rd << 12 | rd << 16 | imm
}

/// `b<cond>` at `from` to `to`; `cond` 14 is always.
pub(in crate::machine::sb) fn bcc(cond: u32, from: u32, to: u32) -> u32 {
    let off = (to.wrapping_sub(from).wrapping_sub(8) as i32 >> 2) as u32 & 0x00FF_FFFF;
    cond << 28 | 0x0A00_0000 | off
}

pub(in crate::machine::sb) fn b(from: u32, to: u32) -> u32 {
    bcc(14, from, to)
}

pub(in crate::machine::sb) fn bl(from: u32, to: u32) -> u32 {
    bcc(14, from, to) | 1 << 24
}

/// Memory with `words` laid out consecutively at each `(address, words)` segment.
pub(in crate::machine::sb) fn mem(segments: &[(u32, &[u32])]) -> Mem {
    let mut m = Mem::new();
    for (addr, words) in segments {
        for (i, w) in words.iter().enumerate() {
            m.write32(addr + 4 * i as u32, *w);
        }
    }
    m
}

pub(super) fn opts() -> BuildOpts {
    BuildOpts::default()
}

pub(super) fn built(m: &mut Mem, o: &BuildOpts) -> Sb {
    let sb = build(m, C, o).expect("a trace");
    check(&sb, m);
    sb
}

/// The terminal uop and its exit.
pub(super) fn tail(sb: &Sb) -> (&Uop, &Exit) {
    let u = sb.uops.last().unwrap();
    (u, &sb.exits[u.exit_idx()])
}

pub(super) fn of_kind(sb: &Sb, kind: u8) -> Vec<&Uop> {
    sb.uops.iter().filter(|u| u.kind == kind).collect()
}

pub(super) fn addrs(sb: &Sb) -> Vec<u32> {
    sb.words.iter().map(|w| w.0).collect()
}

/// Facts that hold for every trace.
pub(in crate::machine::sb) fn check(sb: &Sb, m: &Mem) {
    assert!(sb.uops.len() <= MAX_UOPS && sb.exits.len() <= MAX_EXITS);
    let (last, exit) = tail(sb);
    assert!(
        matches!(
            last.kind,
            k::EXIT | k::LOOP_BACK | k::EXIT_REG | k::EXIT_REG_ARM | k::GENERIC_END
        ),
        "a trace ends in a terminal uop, not {}",
        last.kind
    );
    assert_eq!(sb.n_guest, sb.exits.iter().map(|e| e.done).max().unwrap());
    assert!(exit.done == sb.n_guest, "the terminal path is the longest");
    for u in sb.uops.iter() {
        match u.kind {
            k::GUARD | k::GUARD_REG_EQ | k::LOOP_BACK | k::EXIT | k::EXIT_REG | k::EXIT_REG_ARM => {
                assert!(u.exit_idx() < sb.exits.len(), "exit of uop {}", u.kind);
            }
            k::GENERIC | k::GENERIC_END => assert!(u.exit_idx() < sb.exits.len()),
            kind if k::is_store(kind) => assert_eq!(sb.exits[u.exit_idx()].kind, ExitKind::Stale),
            _ => {}
        }
        if u.kind == k::GENERIC {
            assert_eq!(sb.exits[u.exit_idx()].kind, ExitKind::Stale);
        }
    }
    assert!(sb.exits.iter().all(|e| e.done >= 1 && e.done <= sb.n_guest));
    let mut seen = addrs(sb);
    let n = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), n, "each word once");
    let mut probe = m.clone();
    for (a, w) in sb.words.iter() {
        assert_eq!(probe.read32(*a), *w, "words equals memory at {a:#x}");
        assert!(sb.covers_page(a >> 12));
    }
    assert_eq!(sb.entry, sb.words[0].0);
}

suite! {
    #[test]
    fn a_forward_branch_is_followed_and_the_skipped_words_are_not_read() {
        let mut m = mem(&[(C, &[mov(0, 1), b(C + 4, C + 16), mov(1, 2), mov(2, 3), mov(3, 4), b(C + 20, C + 20)])]);
        let sb = built(&mut m, &opts());
        assert_eq!(addrs(&sb), [C, C + 4, C + 16, C + 20]);
        assert!(of_kind(&sb, k::GUARD).is_empty() && of_kind(&sb, k::LOOP_BACK).is_empty());
        let (u, e) = tail(&sb);
        assert_eq!((u.kind, u.n, u.pc), (k::EXIT, 1, C + 20), "b . on a visited pc ends the trace");
        assert_eq!((e.kind, e.done, e.last_pc, e.target), (ExitKind::Direct, 4, C + 20, C + 20));
        assert_eq!(sb.n_guest, 4);
        assert!(sb.uops.iter().all(|u| u.pc != C + 8 && u.pc != C + 12));
    }

    #[test]
    fn a_loop_back_to_the_entry_guards_the_exit_and_loops() {
        // add r0 ; subs r1 ; bne C ; mov (fall-through, never read)
        let mut m = mem(&[(C, &[add(0, 1), 0xE251_1001, bcc(1, C + 8, C), mov(5, 9)])]);
        let sb = built(&mut m, &opts());
        assert_eq!(addrs(&sb), [C, C + 4, C + 8]);
        let guards = of_kind(&sb, k::GUARD);
        assert_eq!(guards.len(), 1);
        assert_eq!((guards[0].cond, guards[0].n, guards[0].pc), (0, 1, C + 8), "ne is guarded by eq");
        let g = &sb.exits[guards[0].exit_idx()];
        assert_eq!((g.kind, g.done, g.last_pc, g.target), (ExitKind::Direct, 3, C + 8, C + 12));
        let (u, e) = tail(&sb);
        assert_eq!((u.kind, u.n), (k::LOOP_BACK, 0));
        assert_eq!((e.kind, e.done, e.last_pc, e.target), (ExitKind::LoopBack, 3, C + 8, C));
        assert_eq!(sb.n_guest, 3);
    }

    #[test]
    fn an_unconditional_loop_back_carries_the_branch_and_falling_into_the_entry_loops_too() {
        let mut m = mem(&[(C, &[add(0, 1), b(C + 4, C)])]);
        let sb = built(&mut m, &opts());
        let (u, e) = tail(&sb);
        assert_eq!((u.kind, u.n, e.kind, e.done, e.target), (k::LOOP_BACK, 1, ExitKind::LoopBack, 2, C));
        // entry C + 4 (b C): the trace follows to C, runs `mov` and falls into the entry
        let mut m = mem(&[(C, &[mov(0, 1), b(C + 4, C)])]);
        let sb = build(&mut m, C + 4, &opts()).unwrap();
        let (u, e) = tail(&sb);
        assert_eq!((u.kind, u.n, u.pc), (k::LOOP_BACK, 0, C));
        assert_eq!((e.kind, e.done, e.last_pc, e.target), (ExitKind::LoopBack, 2, C, C + 4));
        assert_eq!(addrs(&sb), [C + 4, C]);
    }

    #[test]
    fn a_branch_back_into_the_middle_of_the_trace_ends_with_an_exit_not_an_inner_loop() {
        // mov ; L: sub ; cmp ; bne L ; mov
        let mut m = mem(&[(C, &[mov(0, 3), 0xE240_0001, 0xE350_0000, bcc(1, C + 12, C + 4), mov(9, 1)])]);
        let sb = built(&mut m, &opts());
        assert_eq!(addrs(&sb), [C, C + 4, C + 8, C + 12]);
        let g = of_kind(&sb, k::GUARD);
        assert_eq!((g.len(), g[0].cond), (1, 0));
        assert_eq!(sb.exits[g[0].exit_idx()].target, C + 16, "the loop exit");
        let (u, e) = tail(&sb);
        assert_eq!((u.kind, e.kind, e.target, e.done, e.last_pc), (k::EXIT, ExitKind::Direct, C + 4, 4, C + 12));
        assert!(of_kind(&sb, k::LOOP_BACK).is_empty());
    }

    #[test]
    fn guard_polarity_follows_the_predicted_side_for_every_condition() {
        for cond in 0..14u32 {
            // forward: not taken is followed, the guard leaves when the condition holds
            let mut m = mem(&[
                (C, &[mov(0, 1), bcc(cond, C + 4, C + 100), b(C + 8, C + 8)]),
                (C + 100, &[mov(2, 2), b(C + 104, C + 104)]),
            ]);
            let sb = built(&mut m, &opts());
            let g = of_kind(&sb, k::GUARD);
            assert_eq!(g.len(), 1);
            assert_eq!(u32::from(g[0].cond), cond, "forward branch {cond}");
            let e = &sb.exits[g[0].exit_idx()];
            assert_eq!((e.target, e.done, e.last_pc), (C + 100, 2, C + 4));
            assert_eq!(addrs(&sb), [C, C + 4, C + 8], "the fall-through is followed");
            // backward: taken is followed, the guard is the inverse, the exit the fall-through
            let mut m = mem(&[
                (C + 64, &[mov(0, 1), mov(1, 1), bcc(cond, C + 72, C + 48), mov(3, 3)]),
                (C + 48, &[b(C + 48, C + 48)]),
            ]);
            let sb = build(&mut m, C + 64, &opts()).unwrap();
            check(&sb, &m);
            let g = of_kind(&sb, k::GUARD);
            assert_eq!(u32::from(g[0].cond), cond ^ 1, "backward branch {cond}");
            let e = &sb.exits[g[0].exit_idx()];
            assert_eq!((e.target, e.done, e.last_pc), (C + 76, 3, C + 72), "the fall-through exit");
            assert_eq!(addrs(&sb), [C + 64, C + 68, C + 72, C + 48], "the taken side is followed");
            let (u, e) = tail(&sb);
            assert_eq!((u.kind, e.target, e.done), (k::EXIT, C + 48, 4));
        }
    }

    #[test]
    fn a_flipped_prediction_follows_the_other_side_of_that_branch_only() {
        let (t1, t2) = (C + 0x100, C + 0x200);
        let prog = [mov(0, 1), bcc(1, C + 4, t1), mov(1, 1), bcc(0, C + 12, t2), b(C + 16, C + 16)];
        let mut m = mem(&[(C, &prog), (t1, &[mov(7, 7), b(t1 + 4, t1 + 4)]), (t2, &[mov(8, 8)])]);
        let plain = built(&mut m, &opts());
        assert_eq!(addrs(&plain), [C, C + 4, C + 8, C + 12, C + 16]);
        let flipped = build_flipped(&mut m, C, &opts(), &[C + 4]).unwrap();
        check(&flipped, &m);
        assert_eq!(addrs(&flipped), [C, C + 4, t1, t1 + 4]);
        let g = of_kind(&flipped, k::GUARD);
        assert_eq!(g[0].cond, 0, "ne followed taken: the guard is eq");
        assert_eq!(flipped.exits[g[0].exit_idx()].target, C + 8);
        // flipping addresses that are not conditional branches changes nothing
        let same = build_flipped(&mut m, C, &opts(), &[C, C + 8, C + 16, C + 99]).unwrap();
        assert_eq!((same.uops.len(), addrs(&same)), (plain.uops.len(), addrs(&plain)));
        // both conditional branches flipped
        let both = build_flipped(&mut m, C, &opts(), &[C + 4, C + 12]).unwrap();
        assert_eq!(addrs(&both), [C, C + 4, t1, t1 + 4], "the first flip already left the path");
    }

    #[test]
    fn bl_inlines_the_callee_and_guards_its_return() {
        // mov ; bl f ; mov ; bx r5      f: mov ; bx lr
        let f = C + 0x20;
        let mut m = mem(&[(C, &[mov(1, 1), bl(C + 4, f), mov(2, 2), 0xE12F_FF15]), (f, &[mov(0, 5), BX_LR])]);
        let sb = built(&mut m, &opts());
        assert_eq!(addrs(&sb), [C, C + 4, f, f + 4, C + 8, C + 12]);
        let link: Vec<&Uop> = sb.uops.iter().filter(|u| u.kind == k::MOV_IMM && u.rd == 14).collect();
        assert_eq!((link.len(), link[0].imm, link[0].pc, link[0].n), (1, C + 8, C + 4, 1));
        let g = of_kind(&sb, k::GUARD_REG_EQ);
        assert_eq!((g.len(), g[0].rn, g[0].imm, g[0].pc, g[0].n), (1, 14, C + 8, f + 4, 1));
        let e = &sb.exits[g[0].exit_idx()];
        assert_eq!((e.kind, e.reg, e.done, e.last_pc), (ExitKind::Indirect, 14, 4, f + 4));
        let (u, e) = tail(&sb);
        assert_eq!((u.kind, u.rn), (k::EXIT_REG, 5));
        assert_eq!((e.kind, e.reg, e.done, e.last_pc), (ExitKind::Indirect, 5, 6, C + 12));
        let pos = |kind: u8| sb.uops.iter().position(|u| u.kind == kind).unwrap();
        assert!(pos(k::GUARD_REG_EQ) < pos(k::EXIT_REG));
        assert_eq!(sb.n_guest, 6);
    }

    #[test]
    fn a_mov_pc_lr_return_is_guarded_with_the_no_interwork_exit_and_calls_in_sequence_both_inline() {
        // bl f ; bl g ; bx r4      f: mov pc,lr      g: mov r0,#1 ; mov pc,lr
        let (f, g) = (C + 0x40, C + 0x80);
        let mut m = mem(&[(C, &[bl(C, f), bl(C + 4, g), 0xE12F_FF14]), (f, &[MOV_PC_LR]), (g, &[mov(0, 1), MOV_PC_LR])]);
        let o = BuildOpts { inline_depth: 1, ..opts() };
        let sb = built(&mut m, &o);
        let guards = of_kind(&sb, k::GUARD_REG_EQ);
        assert_eq!(guards.iter().map(|u| u.imm).collect::<Vec<_>>(), [C + 4, C + 8]);
        for u in guards {
            assert_eq!(sb.exits[u.exit_idx()].kind, ExitKind::IndirectArm);
        }
        assert_eq!(addrs(&sb), [C, f, C + 4, g, g + 4, C + 8]);
        assert_eq!(tail(&sb).0.kind, k::EXIT_REG);
    }

    #[test]
    fn the_inline_depth_caps_nested_calls_and_zero_never_inlines() {
        // C: bl f ; bx r5     f: bl g ; bx lr     g: mov ; bx lr
        let (f, g) = (C + 0x40, C + 0x80);
        let prog = [
            (C, &[bl(C, f), 0xE12F_FF15][..]),
            (f, &[bl(f, g), BX_LR][..]),
            (g, &[mov(0, 1), BX_LR][..]),
        ];
        let cases = [
            (0u8, 0usize, k::EXIT, ExitKind::Direct, f),
            (1, 0, k::EXIT, ExitKind::Direct, g),
            (2, 2, k::EXIT_REG, ExitKind::Indirect, 0),
        ];
        for (depth, guards, last, exit, target) in cases {
            let mut m = mem(&prog);
            let o = BuildOpts { inline_depth: depth, ..opts() };
            let sb = built(&mut m, &o);
            assert_eq!(of_kind(&sb, k::GUARD_REG_EQ).len(), guards, "depth {depth}");
            let (u, e) = tail(&sb);
            assert_eq!((u.kind, e.kind, e.target), (last, exit, target), "depth {depth}");
        }
        let mut m = mem(&prog);
        let o = BuildOpts { inline_depth: 0, ..opts() };
        let sb = built(&mut m, &o);
        assert_eq!((addrs(&sb), sb.n_guest), (vec![C], 1), "the callee is not read");
        assert_eq!(sb.uops.iter().filter(|u| u.kind == k::MOV_IMM && u.rd == 14).count(), 1);
    }

    #[test]
    fn indirect_jumps_end_the_trace_with_the_right_exit_kind_and_conditional_ones_guard() {
        for (word, kind, exit) in [
            (0xE12F_FF13, k::EXIT_REG, ExitKind::Indirect),         // bx r3
            (0xE1A0_F004, k::EXIT_REG_ARM, ExitKind::IndirectArm),  // mov pc,r4
        ] {
            let mut m = mem(&[(C, &[mov(0, 1), word])]);
            let sb = built(&mut m, &opts());
            let (u, e) = tail(&sb);
            assert_eq!((u.kind, u.n, e.kind, e.done, e.last_pc), (kind, 1, exit, 2, C + 4));
            assert_eq!(e.reg, u.rn);
            // the same under a condition: a guard leaves through the register, the trace goes on
            let mut m = mem(&[(C, &[mov(0, 1), word & 0x0FFF_FFFF, mov(1, 1), b(C + 12, C + 12)])]);
            let sb = built(&mut m, &opts());
            let g = of_kind(&sb, k::GUARD);
            assert_eq!((g.len(), g[0].cond, g[0].pc), (1, 0, C + 4));
            let e = &sb.exits[g[0].exit_idx()];
            assert_eq!((e.kind, e.done, e.last_pc), (exit, 2, C + 4));
            assert_eq!(addrs(&sb), [C, C + 4, C + 8, C + 12], "the fall-through is followed");
        }
    }

    #[test]
    fn everything_the_builder_cannot_lower_ends_in_generic_end_with_the_word() {
        for word in [
            0xE591_F000, // ldr pc,[r1]
            0xE8BD_8010, // ldmfd sp!,{r4,pc}
            0x0B00_0010, // bleq
            0xEF00_0000, // swi
            0xE12F_FF33, // blx r3
            0xFA00_0000, // blx label
            0xE12F_FF1F, // bx pc
            0xE1A0_F00F, // mov pc,pc
            0xE1B0_F00E, // movs pc,lr
            0xE10F_3000, // mrs r3,cpsr
            0xE1A0_F11E, // mov pc,lr,lsl r1
            0xE1A0_F10E, // mov pc,lr,lsl #2
            0xE28F_F004, // add pc,pc,#4
        ] {
            let mut m = mem(&[(C, &[mov(0, 1), word, mov(1, 1)])]);
            let sb = built(&mut m, &opts());
            let (u, e) = tail(&sb);
            assert_eq!((u.kind, u.imm, u.pc, u.n), (k::GENERIC_END, word, C + 4, 1), "{word:#x}");
            assert_eq!((e.kind, e.done, e.last_pc), (ExitKind::Dynamic, 2, C + 4), "{word:#x}");
            assert_eq!(addrs(&sb), [C, C + 4], "{word:#x}: nothing after an interpreter-run ender");
        }
    }
}
