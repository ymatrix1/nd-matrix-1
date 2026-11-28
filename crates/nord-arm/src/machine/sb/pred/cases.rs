//! OWNER: W2-PEEP (tests). The decisions of the predicate grouping on hand-built uop sequences:
//! what merges, what ends a run, and the skip count of the merged predicate.

use super::{run_counted, Grouped, MAX_RUN};
use crate::machine::sb::trace::testexec::suite;
use crate::machine::sb::trace::BuildOpts;
use crate::machine::sb::uop::{k, Uop};

fn on() -> BuildOpts {
    BuildOpts {
        predicate: true,
        ..BuildOpts::default()
    }
}

/// A uop of `kind` at word `i`; `rd` tells instances apart.
fn at(kind: u8, i: u32) -> Uop {
    let mut u = Uop::new(kind, 0x1000 + 4 * i);
    u.rd = i as u8;
    u
}

fn pred(cond: u8, skip: u32, i: u32) -> Uop {
    let mut p = Uop::new(k::PRED, 0x1000 + 4 * i);
    (p.n, p.cond, p.imm) = (0, cond, skip);
    p
}

/// `PRED cond,1 ; ADD_IMM` as `wrap_pred` makes it for one conditional add at word `i`.
fn cond_add(cond: u8, i: u32) -> [Uop; 2] {
    [pred(cond, 1, i), at(k::ADD_IMM, i)]
}

fn flat(parts: &[&[Uop]]) -> Vec<Uop> {
    parts.iter().flat_map(|p| p.iter().copied()).collect()
}

/// `(kind, cond, imm)` of every uop: enough to see the structure.
fn shape(uops: &[Uop]) -> Vec<(u8, u8, u32)> {
    uops.iter().map(|u| (u.kind, u.cond, u.imm)).collect()
}

fn group(mut uops: Vec<Uop>) -> (Vec<Uop>, Grouped) {
    let g = run_counted(&mut uops, &on());
    (uops, g)
}

suite! {
    #[test]
    fn adjacent_predicates_of_one_condition_become_one_with_the_combined_skip_count() {
        let (a, b, c) = (cond_add(0, 0), cond_add(0, 1), cond_add(0, 2));
        let (out, g) = group(flat(&[&a, &b, &c, &[Uop::exit(0, 0x100C)]]));
        assert_eq!(g, Grouped { runs: 1, removed: 2 });
        assert_eq!(
            shape(&out),
            [(k::PRED, 0, 3), (k::ADD_IMM, 0, 0), (k::ADD_IMM, 0, 0), (k::ADD_IMM, 0, 0), (k::EXIT, 0, 0)]
        );
        assert_eq!(out[0].pc, 0x1000, "the merged predicate sits at the first instruction");
        assert_eq!(out.iter().filter(|u| u.n == 0).count(), 2, "PRED and EXIT carry n = 0");
        assert_eq!(out.iter().map(|u| u.rd).collect::<Vec<_>>(), [0, 0, 1, 2, 0], "bodies keep their order");
    }

    #[test]
    fn a_body_of_several_uops_counts_all_of_them() {
        let two = [pred(1, 2, 0), at(k::ADD_IMM, 0), at(k::SUB_IMM, 0)];
        let (out, _) = group(flat(&[&two, &cond_add(1, 1), &[Uop::exit(0, 0x1008)]]));
        assert_eq!(out[0].imm, 3);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn a_different_condition_or_a_gap_ends_the_run() {
        let (out, g) = group(flat(&[&cond_add(0, 0), &cond_add(1, 1), &[Uop::exit(0, 0x1008)]]));
        assert_eq!((g, out.len()), (Grouped::default(), 5), "eq then ne");
        let plain = [at(k::ADD_IMM, 1)];
        let (out, g) = group(flat(&[&cond_add(0, 0), &plain, &cond_add(0, 2), &[Uop::exit(0, 0x100C)]]));
        assert_eq!((g, out.len()), (Grouped::default(), 6), "an unconditional word in between");
    }

    #[test]
    fn a_body_that_may_change_the_flags_ends_the_run_after_itself() {
        let writers = [k::dp(1, 4, true), k::CMP_IMM, k::TST_REG, k::GENERIC, k::MSR_C];
        for writer in writers {
            let mut w = at(writer, 1);
            w.n = 1;
            let wrapped = [pred(0, 1, 1), w];
            // writer last: it still joins the run before it
            let (out, g) = group(flat(&[&cond_add(0, 0), &wrapped, &[Uop::exit(0, 0x1008)]]));
            assert_eq!((g.runs, g.removed, out[0].imm), (1, 1, 2), "writer {writer} joins");
            // writer first: nothing may follow it, the later member stays alone
            let (out, g) = group(flat(&[&wrapped, &cond_add(0, 2), &[Uop::exit(0, 0x1008)]]));
            assert_eq!((g, out.len()), (Grouped::default(), 5), "writer {writer} ends");
            // writer in the middle of three: the run is the first two
            let (out, g) = group(flat(&[&cond_add(0, 0), &wrapped, &cond_add(0, 2), &[Uop::exit(0, 0x100C)]]));
            assert_eq!((g.runs, out[0].imm, out.len()), (1, 2, 6), "writer {writer} in the middle");
        }
    }

    #[test]
    fn flag_readers_and_memory_uops_do_not_end_a_run() {
        for kind in [k::MRS, k::LDR_IMM, k::STR_IMM, k::dp(1, 4, false), k::GENERIC] {
            let mut body = at(kind, 0);
            body.n = 1;
            let first = [pred(2, 1, 0), body];
            // GENERIC may write flags: it can only be the last member
            let (_, g) = group(flat(&[&first, &cond_add(2, 1), &[Uop::exit(0, 0x1008)]]));
            let expect_two = kind != k::GENERIC;
            assert_eq!(g.runs, u32::from(expect_two), "kind {kind} first of two");
            let (_, g) = group(flat(&[&cond_add(2, 0), &first, &[Uop::exit(0, 0x1008)]]));
            assert_eq!(g.runs, 1, "kind {kind} last of two");
        }
    }

    #[test]
    fn a_body_with_a_control_uop_is_never_merged() {
        for kind in [k::GUARD, k::EXIT, k::LOOP_BACK, k::GENERIC_END, k::PRED, k::CMP_GUARD_I, k::MOV_PAIR] {
            let ctl = [pred(0, 1, 0), at(kind, 0)];
            let before = flat(&[&ctl, &cond_add(0, 1), &[Uop::exit(0, 0x1008)]]);
            let (out, g) = group(before.clone());
            assert_eq!((g, &out), (Grouped::default(), &before), "kind {kind} first");
            let before = flat(&[&cond_add(0, 0), &ctl, &[Uop::exit(0, 0x1008)]]);
            let (out, g) = group(before.clone());
            assert_eq!((g, &out), (Grouped::default(), &before), "kind {kind} second");
        }
    }

    #[test]
    fn a_run_has_at_most_eight_members() {
        let mut uops = Vec::new();
        for i in 0..(MAX_RUN as u32 + 2) {
            uops.extend(cond_add(0, i));
        }
        uops.push(Uop::exit(0, 0x2000));
        let (out, g) = group(uops);
        // the ninth and tenth are a run of two of their own
        assert_eq!(g, Grouped { runs: 2, removed: MAX_RUN as u32 - 1 + 1 });
        assert_eq!((out[0].kind, out[0].imm), (k::PRED, MAX_RUN as u32));
        assert_eq!((out[MAX_RUN + 1].kind, out[MAX_RUN + 1].imm), (k::PRED, 2));
        assert_eq!(out.len(), 1 + MAX_RUN + 1 + 2 + 1);
    }

    #[test]
    fn a_predicate_that_runs_past_the_trace_is_left_alone() {
        let before = vec![pred(0, 5, 0), at(k::ADD_IMM, 0), Uop::exit(0, 0x1004)];
        let (out, g) = group(before.clone());
        assert_eq!((out, g), (before, Grouped::default()));
    }

    #[test]
    fn with_the_switch_off_nothing_is_grouped() {
        let before = flat(&[&cond_add(0, 0), &cond_add(0, 1), &[Uop::exit(0, 0x1008)]]);
        let mut uops = before.clone();
        let g = run_counted(&mut uops, &BuildOpts::default());
        assert_eq!((uops, g), (before, Grouped::default()));
    }
}
