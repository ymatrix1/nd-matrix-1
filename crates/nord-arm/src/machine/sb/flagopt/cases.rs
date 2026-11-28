//! OWNER: W2-PEEP (tests). The decisions of the dead-flag pass on hand-built uop sequences: one
//! case per rule of the liveness analysis, so each mutation site has a test that names it.

use super::{run_counted, Dropped};
use crate::machine::sb::trace::testexec::suite;
use crate::machine::sb::trace::BuildOpts;
use crate::machine::sb::uop::{k, Uop};

fn on() -> BuildOpts {
    BuildOpts {
        dead_flags: true,
        ..BuildOpts::default()
    }
}

/// A generic data-processing uop (`rd = 1`, `rn = 2`, `rm = 3`) at word `i` of the trace.
fn dp(form: u32, op: u32, s: bool, sh: u8, i: u32) -> Uop {
    let mut u = Uop::new(k::dp(form, op, s), 0x1000 + 4 * i);
    (u.rd, u.rn, u.rm, u.sh) = (1, 2, 3, sh);
    u
}

fn adds(i: u32) -> Uop {
    dp(1, 4, true, 0, i)
}

fn at(kind: u8, i: u32) -> Uop {
    Uop::new(kind, 0x1000 + 4 * i)
}

fn pred(cond: u8, skip: u32, i: u32) -> Uop {
    let mut p = at(k::PRED, i);
    (p.n, p.cond, p.imm) = (0, cond, skip);
    p
}

fn exit(i: u32) -> Uop {
    Uop::exit(0, 0x1000 + 4 * i)
}

/// Run the pass on `uops`; the kinds afterwards and what it did.
fn after(mut uops: Vec<Uop>) -> (Vec<u8>, Dropped) {
    let d = run_counted(&mut uops, &on());
    (uops.iter().map(|u| u.kind).collect(), d)
}

/// The kind of `u` once its S bit is gone.
const fn twin(u: &Uop) -> u8 {
    u.kind - 1
}

suite! {
    #[test]
    fn a_definition_that_a_later_one_overwrites_is_dropped_and_the_last_one_is_live_at_the_exit() {
        let (k1, d) = after(vec![adds(0), adds(1), exit(1)]);
        assert_eq!(k1, [twin(&adds(0)), adds(1).kind, k::EXIT]);
        assert_eq!(d, Dropped { s_cleared: 1, compares_removed: 0 });
    }

    #[test]
    fn every_kind_of_barrier_keeps_the_definition_in_front_of_it() {
        // the second `adds` overwrites everything, so only the barrier can keep the first one
        for barrier in [
            Uop::exit(0, 0x1004),
            at(k::GUARD, 1),
            at(k::GUARD_REG_EQ, 1),
            at(k::GENERIC, 1),
            at(k::GENERIC_END, 1),
            at(k::MRS, 1),
            at(k::MSR_C, 1),
            at(k::STR_IMM, 1),
            at(k::STM, 1),
            at(k::LOOP_BACK, 1),
            at(k::EXIT_REG, 1),
            at(k::CMP_GUARD_I, 1),
        ] {
            let (ks, d) = after(vec![adds(0), barrier, adds(2), exit(2)]);
            assert_eq!(ks[0], adds(0).kind, "kept in front of kind {}", barrier.kind);
            assert_eq!(d.s_cleared, 0, "kind {}", barrier.kind);
        }
        // a load is not a barrier: nothing can end the trace there
        let (ks, d) = after(vec![adds(0), at(k::LDR_IMM, 1), adds(2), exit(2)]);
        assert_eq!((ks[0], d.s_cleared), (twin(&adds(0)), 1));
    }

    #[test]
    fn code_after_an_exit_does_not_make_the_exit_transparent() {
        // dead code behind a terminal uop: the exit still needs the flags
        let (ks, _) = after(vec![adds(0), exit(0), adds(1), exit(1)]);
        assert_eq!(ks[0], adds(0).kind);
    }

    #[test]
    fn a_flag_read_after_the_definition_keeps_it() {
        // subs ; adc (reads C) ; cmp (overwrites all) ; exit: only the carry-in keeps the subs
        let subs = dp(1, 2, true, 0, 0);
        let (a, _) = after(vec![subs, dp(1, 5, false, 0, 1), at(k::CMP_IMM, 2), exit(2)]);
        assert_eq!(a[0], subs.kind, "adc reads C");
        let (b, _) = after(vec![subs, dp(1, 4, false, 0, 1), at(k::CMP_IMM, 2), exit(2)]);
        assert_eq!(b[0], twin(&subs), "add does not");
        for op in [6, 7] {
            let (c, _) = after(vec![subs, dp(1, op, false, 0, 1), at(k::CMP_IMM, 2), exit(2)]);
            assert_eq!(c[0], subs.kind, "op {op} reads C");
        }
        // mov r1, r3, rrx reads C without S; lsl and lsr do not
        let (d, _) = after(vec![subs, dp(1, 13, false, 3 << 5, 1), at(k::CMP_IMM, 2), exit(2)]);
        assert_eq!(d[0], subs.kind, "rrx reads C");
        let (e, _) = after(vec![subs, dp(1, 13, false, 1 << 5 | 3, 1), at(k::CMP_IMM, 2), exit(2)]);
        assert_eq!(e[0], twin(&subs), "lsr #3 does not");
    }

    #[test]
    fn the_condition_of_a_predicate_is_a_read() {
        // subs ; PRED eq,1 ; mov ; cmp ; exit: the predicate reads Z, nothing else does
        let subs = dp(1, 2, true, 0, 0);
        let body = at(k::MOV_IMM, 2);
        let (a, _) = after(vec![subs, pred(0, 1, 1), body, at(k::CMP_IMM, 3), exit(3)]);
        assert_eq!(a[0], subs.kind, "eq reads Z");
    }

    #[test]
    fn the_skip_path_of_a_predicate_keeps_what_only_it_needs() {
        // tst r2,r3 (writes N and Z) ; PRED vs,1 ; cmp (body overwrites all) ; guard ; exit:
        // the body path and the predicate read neither N nor Z, the skip path reaches the guard
        let tst = at(k::TST_REG, 0);
        let (ks, d) = after(vec![tst, pred(6, 1, 1), at(k::CMP_IMM, 2), at(k::GUARD, 3), exit(3)]);
        assert_eq!(ks[0], k::TST_REG, "live on the skip path");
        assert_eq!(d.compares_removed, 0);
    }

    #[test]
    fn a_definition_inside_a_body_is_judged_on_the_path_where_it_runs() {
        // PRED eq,2 ; adds ; adds ; exit: the first adds of the body is overwritten on the only
        // path that runs it
        let (a, d) = after(vec![pred(0, 2, 0), adds(1), adds(2), exit(2)]);
        assert_eq!(a[1], twin(&adds(1)));
        assert_eq!(d.s_cleared, 1);
        // PRED eq,1 ; adds ; adds ; exit: the body is one uop, so the skip path keeps the old flags
        // live: the first adds (outside the body) still feeds the exit when the predicate fails
        let (b, _) = after(vec![adds(0), pred(0, 1, 1), adds(2), exit(3)]);
        assert_eq!(b[0], adds(0).kind, "the skip path reaches the exit");
    }

    #[test]
    fn only_the_flags_an_operation_may_change_are_checked() {
        // adds (all four) ; ands #rot (N, Z and C, keeps V) ; exit: V of the adds is observed at
        // the exit, so the adds stays although N, Z and C are overwritten
        let ands = {
            let mut u = dp(0, 0, true, 0, 1);
            u.imm = 0x8000_0000;
            u
        };
        let (ks, _) = after(vec![dp(0, 4, true, 0, 0), ands, exit(1)]);
        assert_eq!(ks[0], dp(0, 4, true, 0, 0).kind, "V is live");
        // the same with a compare that overwrites V too: now the first adds is dead
        let (ks, _) = after(vec![adds(0), at(k::CMP_REG, 1), exit(1)]);
        assert_eq!(ks[0], twin(&adds(0)));
    }

    #[test]
    fn a_logic_operation_passes_the_old_carry_on_unless_its_shifter_makes_one() {
        let subs = dp(1, 2, true, 0, 0);
        let live_c = |op: Uop| {
            // subs ; <op> ; adc (reads C) ; cmp ; exit
            after(vec![subs, op, dp(1, 5, false, 0, 2), at(k::CMP_IMM, 3), exit(3)]).0[0] == subs.kind
        };
        assert!(live_c(dp(1, 0, true, 0, 1)), "ands r, lsl #0: the carry passes through");
        assert!(live_c(dp(2, 13, true, 0, 1)), "movs by register: amount 0 passes it too");
        assert!(live_c(dp(0, 13, true, 1, 1)), "movs #imm without rotation passes it");
        assert!(!live_c(dp(0, 13, true, 0, 1)), "movs #imm ror: bit 31 of the immediate");
        assert!(!live_c(dp(1, 4, true, 0, 1)), "adds overwrites C");
    }

    #[test]
    fn a_compare_whose_flags_are_dead_becomes_the_no_op_that_still_counts_its_instruction() {
        let cmp = at(k::CMP_IMM, 0);
        let mut uops = vec![cmp, at(k::CMP_REG, 1), exit(1)];
        let d = run_counted(&mut uops, &on());
        assert_eq!(uops[0], Uop::new(k::MOV_REG, 0x1000), "r0 = r0, n = 1, pc kept");
        assert_eq!(uops[1].kind, k::CMP_REG);
        assert_eq!(d, Dropped { s_cleared: 0, compares_removed: 1 });
        // the generic compare ids (tst teq cmp cmn) have no twin either
        let mut uops = vec![dp(1, 10, true, 0, 0), adds(1), exit(1)];
        run_counted(&mut uops, &on());
        assert_eq!(uops[0], Uop::new(k::MOV_REG, 0x1000));
    }

    #[test]
    fn nothing_else_changes_and_a_second_run_finds_nothing_more() {
        let before = vec![adds(0), at(k::LDR_IMM, 1), adds(2), pred(0, 1, 3), at(k::MOV_IMM, 3), exit(4)];
        let mut uops = before.clone();
        run_counted(&mut uops, &on());
        assert_eq!(uops.len(), before.len());
        for (a, b) in uops.iter().zip(&before) {
            assert_eq!((a.n, a.pc, a.rd, a.rn, a.rm, a.imm), (b.n, b.pc, b.rd, b.rn, b.rm, b.imm));
        }
        assert_eq!(run_counted(&mut uops, &on()), Dropped::default());
    }

    #[test]
    fn with_the_switch_off_nothing_is_dropped() {
        let mut uops = vec![adds(0), adds(1), exit(1)];
        let before = uops.clone();
        let d = run_counted(&mut uops, &BuildOpts::default());
        assert_eq!((uops, d), (before, Dropped::default()));
    }
}
