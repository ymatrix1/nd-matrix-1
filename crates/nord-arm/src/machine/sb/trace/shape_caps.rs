//! OWNER: W1-FLOW (tests). Shape tests of `trace::build` that concern limits and bookkeeping:
//! unmapped and hooked pages (I9), the page, guard, uop and exit-table caps, `n_guest`, the early
//! exits of stores, and the roll-back of an instruction that crosses a cap. The helpers are those
//! of `shape.rs`.

use super::super::lower_flow::mutant;
use super::super::uop::{k, Exit, ExitKind, MAX_EXITS, MAX_UOPS};
use super::shape::{add, addrs, b, bcc, bl, built, check, mem, mov, of_kind, opts, tail, BX_LR, C};
use super::testexec::suite;
use super::{build, build_flipped, BuildOpts, Builder, Step};
use crate::mem::HOOK_X;

suite! {
    #[test]
    fn a_trace_stops_in_front_of_an_unmapped_or_hooked_page_and_maps_nothing() {
        let end = C + 0xFF8;
        for hooked in [false, true] {
            let mut m = mem(&[(end, &[mov(0, 1), mov(1, 2)])]);
            if hooked {
                m.write32(C + 0x1000, 0);
                m.add_flag(C + 0x1000, C + 0x1000, HOOK_X);
            }
            let count = m.auto_mapped_count;
            let sb = build(&mut m, end, &opts()).unwrap();
            check(&sb, &m);
            let (u, e) = tail(&sb);
            assert_eq!((u.kind, e.kind, e.target, e.done, e.last_pc), (k::EXIT, ExitKind::Direct, C + 0x1000, 2, end + 4));
            assert_eq!(addrs(&sb), [end, end + 4]);
            assert_eq!(m.auto_mapped_count, count, "I9: nothing is mapped (hooked {hooked})");
            assert!(m.is_mapped(C + 0x1000) == hooked);
        }
        // branch targets, taken or inlined
        for far in [C + 0x3000, C + 0x2000] {
            let mut m = mem(&[(C, &[mov(0, 1), b(C + 4, far)]), (C + 0x2000, &[mov(5, 5)])]);
            m.add_flag(C + 0x2000, C + 0x2000, HOOK_X);
            let sb = built(&mut m, &opts());
            let (u, e) = tail(&sb);
            assert_eq!((u.kind, u.n, e.target, e.done), (k::EXIT, 1, far, 2));
            let mut m2 = mem(&[(C, &[bl(C, far)]), (C + 0x2000, &[mov(5, 5)])]);
            m2.add_flag(C + 0x2000, C + 0x2000, HOOK_X);
            let sb = built(&mut m2, &opts());
            assert_eq!((sb.uops.len(), sb.uops[0].kind, sb.uops[0].imm), (2, k::MOV_IMM, C + 4));
            assert_eq!((tail(&sb).1.target, tail(&sb).1.done), (far, 1));
            assert_eq!(addrs(&sb), [C], "the callee is not read");
        }
        // a taken conditional branch to unmapped code: guard, then an exit
        let mut m = mem(&[(C, &[mov(0, 1), bcc(1, C + 4, C + 0x3000), mov(1, 1), b(C + 12, C + 12)])]);
        let o = &opts();
        let sb = build_flipped(&mut m, C, o, &[C + 4]).unwrap();
        check(&sb, &m);
        let g = of_kind(&sb, k::GUARD);
        assert_eq!((g[0].cond, sb.exits[g[0].exit_idx()].target), (0, C + 8));
        assert_eq!((tail(&sb).1.target, tail(&sb).1.done, addrs(&sb).len()), (C + 0x3000, 2, 2));
        // an inlined return into an unmapped page: the guard, then an exit
        let f = C + 0x100;
        let mut m = mem(&[(C + 0xFFC, &[bl(C + 0xFFC, f)]), (f, &[BX_LR])]);
        let sb = build(&mut m, C + 0xFFC, &opts()).unwrap();
        check(&sb, &m);
        assert_eq!(of_kind(&sb, k::GUARD_REG_EQ).len(), 1);
        assert_eq!((tail(&sb).0.kind, tail(&sb).1.target, tail(&sb).1.done), (k::EXIT, C + 0x1000, 2));
    }

    #[test]
    fn the_page_cap_counts_code_pages_and_ends_the_trace_at_the_next_new_one() {
        let (p1, p2) = (C + 0x1000, C + 0x2000);
        let prog = [(C, &[b(C, p1)][..]), (p1, &[b(p1, p2)][..]), (p2, &[mov(0, 1), b(p2 + 4, p2 + 4)][..])];
        for (cap, pages, target) in [(1usize, 1u8, p1), (2, 2, p2), (3, 3, p2 + 4), (9, 3, p2 + 4)] {
            let mut m = mem(&prog);
            let o = BuildOpts { max_pages: cap, ..opts() };
            let sb = built(&mut m, &o);
            assert_eq!(sb.n_pages, pages, "cap {cap}");
            assert_eq!((tail(&sb).0.kind, tail(&sb).1.target), (k::EXIT, target), "cap {cap}");
            assert!(sb.covers_page(C >> 12));
        }
    }

    #[test]
    fn the_guard_cap_turns_the_next_conditional_branch_into_a_generic_end() {
        // four forward conditional branches, each skipping one word
        let prog: Vec<u32> = (0..4).map(|i| bcc(0, C + 8 * i, C + 8 * i + 16)).flat_map(|w| [w, mov(1, 1)]).collect();
        for (cap, guards) in [(0usize, 0usize), (1, 1), (2, 2), (6, 4)] {
            let mut m = mem(&[(C, &prog)]);
            let o = BuildOpts { max_guards: cap, ..opts() };
            let sb = built(&mut m, &o);
            assert_eq!(of_kind(&sb, k::GUARD).len(), guards, "cap {cap}");
            if guards < 4 {
                let (u, e) = tail(&sb);
                assert_eq!((u.kind, u.imm, e.kind), (k::GENERIC_END, prog[2 * guards], ExitKind::Dynamic), "cap {cap}");
            }
        }
        // the same cap bounds inlined returns: the return becomes a plain indirect exit
        let f = C + 0x40;
        let mut m = mem(&[(C, &[bl(C, f), mov(1, 1)]), (f, &[BX_LR])]);
        let o = BuildOpts { max_guards: 0, ..opts() };
        let sb = built(&mut m, &o);
        assert_eq!((of_kind(&sb, k::GUARD_REG_EQ).len(), tail(&sb).0.kind), (0, k::EXIT_REG));
    }

    #[test]
    fn the_uop_cap_ends_a_straight_line_after_cap_minus_one_instructions() {
        let run: Vec<u32> = (0..300).map(|_| add(0, 1)).collect();
        for cap in [1usize, 2, 3, 5] {
            let mut m = mem(&[(C, &run)]);
            let o = BuildOpts { max_uops: cap, ..opts() };
            let sb = built(&mut m, &o);
            let body = cap.saturating_sub(1).max(1);
            assert_eq!((sb.n_guest as usize, sb.uops.len()), (body, body + 1), "cap {cap}");
            let (u, e) = tail(&sb);
            assert_eq!((u.kind, u.n, e.kind, e.target), (k::EXIT, 0, ExitKind::Direct, C + 4 * body as u32));
            assert_eq!((e.done as usize, e.last_pc), (body, C + 4 * body as u32 - 4));
            assert_eq!(sb.words.len(), body);
        }
        // the cap of 128 (the default): the trace runs to a hard limit, uops or exit table
        for o in [opts(), BuildOpts { max_uops: 128, ..opts() }] {
            let sb = built(&mut mem(&[(C, &run)]), &o);
            assert!(sb.uops.len() <= MAX_UOPS && sb.exits.len() <= MAX_EXITS && sb.n_guest >= 60);
            assert!(
                sb.uops.len() == MAX_UOPS || sb.exits.len() >= MAX_EXITS - 1,
                "ended early: {} uops, {} exits",
                sb.uops.len(),
                sb.exits.len()
            );
            assert_eq!(tail(&sb).1.kind, ExitKind::Direct);
        }
        // a cap above the hard one is clamped
        let o = BuildOpts { max_uops: 100_000, ..opts() };
        assert!(built(&mut mem(&[(C, &run)]), &o).uops.len() <= MAX_UOPS);
    }

    #[test]
    fn the_exit_table_cap_ends_a_trace_full_of_stores() {
        let run: Vec<u32> = (0..200).map(|_| 0xE588_0000).collect(); // str r0,[r8]
        let mut m = mem(&[(C, &run)]);
        let sb = built(&mut m, &opts());
        assert!(sb.exits.len() <= MAX_EXITS && sb.uops.len() <= MAX_UOPS);
        let stale: Vec<&Exit> = sb.exits.iter().filter(|e| e.kind == ExitKind::Stale).collect();
        assert_eq!(stale.len(), sb.n_guest as usize, "one early exit per store");
        for (i, e) in stale.iter().enumerate() {
            assert_eq!((e.done as usize, e.last_pc, e.target), (i + 1, C + 4 * i as u32, C + 4 * i as u32 + 4));
        }
        assert!(sb.n_guest >= 60 && sb.n_guest < 200);
    }

    #[test]
    fn n_guest_is_the_longest_path_and_the_snapshot_is_the_memory_of_the_path() {
        // a trace with side exits: forward guards drop out early, the last path is the longest
        let prog = [
            mov(0, 1),
            bcc(1, C + 4, C + 200),
            mov(1, 2),
            bcc(0, C + 12, C + 300),
            add(2, 1),
            b(C + 20, C + 28),
            mov(9, 9),
            mov(3, 3),
            b(C + 32, C + 32),
        ];
        let mut m = mem(&[(C, &prog)]);
        let sb = built(&mut m, &opts());
        let dones: Vec<u16> = sb.exits.iter().map(|e| e.done).collect();
        assert_eq!(sb.n_guest, *dones.iter().max().unwrap());
        assert_eq!(addrs(&sb), [C, C + 4, C + 8, C + 12, C + 16, C + 20, C + 28, C + 32]);
        assert_eq!(sb.n_guest as usize, sb.words.len(), "one path, no literal: every word is an instruction");
        for g in of_kind(&sb, k::GUARD) {
            assert!(sb.exits[g.exit_idx()].done < sb.n_guest);
        }
    }

    #[test]
    fn stores_and_generics_get_one_stale_exit_each_with_static_counts() {
        // mov ; str ; ldm/stm via r9 ; b .
        let mut m = mem(&[(C, &[mov(0, 1), 0xE588_0000, 0xE889_0003, b(C + 12, C + 12)])]);
        let sb = built(&mut m, &opts());
        let stale: Vec<&Exit> = sb.exits.iter().filter(|e| e.kind == ExitKind::Stale).collect();
        assert!(stale.iter().any(|e| (e.done, e.last_pc, e.target) == (2, C + 4, C + 8)), "{stale:?}");
        assert!(stale.iter().any(|e| (e.done, e.last_pc, e.target) == (3, C + 8, C + 12)), "{stale:?}");
        for u in sb.uops.iter().filter(|u| k::is_store(u.kind) || u.kind == k::GENERIC) {
            let e = &sb.exits[u.exit_idx()];
            assert_eq!((e.kind, e.last_pc, e.target), (ExitKind::Stale, u.pc, u.pc + 4));
        }
    }

    #[test]
    fn an_instruction_over_the_cap_is_rolled_back_whole_and_the_trace_ends_in_front_of_it() {
        let mut m = mem(&[(C, &[add(0, 1), add(1, 1), add(2, 1)])]);
        let o = opts();
        let mut b = Builder {
            opts: &o,
            flips: &[],
            start: C,
            limit: 3,
            max_pages: 4,
            uops: Vec::new(),
            exits: Vec::new(),
            words: Vec::new(),
            pages: vec![C >> 12],
            visited: Vec::new(),
            depth: 0,
            rets: vec![0x1234],
            done: 0,
            last_pc: C,
        };
        assert!(matches!(b.step(&mut m, C), Step::Next(n) if n == C + 4));
        assert!(matches!(b.step(&mut m, C + 4), Step::Next(_)));
        let before = (b.uops.len(), b.exits.len(), b.words.len(), b.visited.len(), b.done, b.last_pc);
        // the third instruction needs a slot and leaves none for the terminal: rolled back
        assert!(matches!(b.step(&mut m, C + 8), Step::Overflow));
        assert_eq!((b.uops.len(), b.exits.len(), b.words.len(), b.visited.len(), b.done, b.last_pc), before);
        assert_eq!(b.rets, [0x1234]);
        assert!(b.words.iter().all(|w| w.0 != C + 8) && !b.visited.contains(&(C + 8)));
    }

    #[test]
    fn a_rolled_back_call_restores_the_depth_and_the_return_stack() {
        let mut m = mem(&[(C, &[bl(C, C + 0x40)]), (C + 0x40, &[BX_LR])]);
        let o = opts();
        let make = |rets: Vec<u32>, depth: u8| Builder {
            opts: &o,
            flips: &[],
            start: C + 0x80,
            limit: 128,
            max_pages: 4,
            uops: Vec::new(),
            exits: Vec::new(),
            words: Vec::new(),
            pages: vec![C >> 12],
            visited: Vec::new(),
            depth,
            rets,
            done: 0,
            last_pc: C,
        };
        // a call pushes, a return pops; restoring puts both back
        let mut b = make(vec![], 0);
        let s = b.snap();
        b.step(&mut m, C);
        assert_eq!((b.depth, b.rets.clone()), (1, vec![C + 4]));
        b.restore(s);
        assert_eq!((b.depth, b.rets.len(), b.uops.len(), b.words.len()), (0, 0, 0, 0));
        let mut b = make(vec![C + 4], 1);
        let s = b.snap();
        b.step(&mut m, C + 0x40);
        assert_eq!((b.depth, b.rets.len()), (0, 0), "the return pops");
        b.restore(s);
        assert_eq!((b.depth, b.rets.clone()), (1, vec![C + 4]));
    }

    #[test]
    fn mutation_sites_do_nothing_when_no_mutant_is_active() {
        assert!(!(200..=215).any(mutant));
    }
}
