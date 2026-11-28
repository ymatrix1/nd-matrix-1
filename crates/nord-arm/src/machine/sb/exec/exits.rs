//! OWNER: W2-XCORE (tests). `run_sb` on hand-built traces: every exit kind with its exact `done`,
//! `last_pc`, target and `cpu.insns`; the loop accumulator under every budget; the predicate
//! skip; a fault and a stale store inside a trace; the watch return; and a fused uop against its
//! unfused expansion on random states.
//!
//! The suite runs unmutated as ordinary tests and whole under each mutant of `exec.rs` and
//! `exec_flow.rs` (`kills.rs`): one failed assertion is a kill.

use super::*;
use crate::machine::sb::exec_flow::fused::tests::random_pair;
use crate::machine::sb::exec_mem::{F_P, F_U};
use crate::machine::sb::testkit::{random_cpu, CpuSnap, LogBus, Rng};
use crate::machine::sb::trace::testexec::suite;
use crate::machine::sb::uop::fused::{expand, fuse_pair};
use crate::machine::sb::uop::Uop;
use crate::machine::SysBus;

const PC: u32 = 0x1000;

fn exit(kind: ExitKind, done: u16, last_pc: u32, target: u32) -> Exit {
    Exit::new(kind, done, last_pc, target)
}

fn trace(uops: Vec<Uop>, exits: Vec<Exit>, n_guest: u16) -> Sb {
    Sb::from_parts(PC, n_guest, uops, exits, vec![(PC, 0)]).unwrap()
}

fn mov(rd: u8, imm: u32, pc: u32) -> Uop {
    let mut u = Uop::new(k::MOV_IMM, pc);
    (u.rd, u.imm) = (rd, imm);
    u
}

fn add(rd: u8, imm: u32, pc: u32) -> Uop {
    let mut u = Uop::new(k::ADD_IMM, pc);
    (u.rd, u.rn, u.imm) = (rd, rd, imm);
    u
}

fn cmp(rn: u8, imm: u32, pc: u32) -> Uop {
    let mut u = Uop::new(k::CMP_IMM, pc);
    (u.rn, u.imm) = (rn, imm);
    u
}

fn control(kind: u8, cond: u8, rs: u8, n: u8, pc: u32) -> Uop {
    let mut u = Uop::new(kind, pc);
    (u.cond, u.rs, u.n) = (cond, rs, n);
    u
}

/// What a run leaves behind: its `RunOut`, the CPU, the bus and the budget still `left`.
type Ran = (RunOut, Cpu, SysBus, u64);

/// `((exit, target), (insns done, left, bus.pc, r15, thumb))` of a [`Ran`].
type Seen = ((u16, u32), (u64, u64, u32, u32, bool));

/// Run `sb` on a fresh machine prepared by `setup`, with `left` instructions of budget.
fn go(sb: &Sb, left: u64, watch: bool, setup: impl FnOnce(&mut Cpu, &mut SysBus)) -> Ran {
    let (mut cpu, mut bus) = (Cpu::new(), SysBus::new());
    cpu.insns = 100;
    setup(&mut cpu, &mut bus);
    let mut left_now = left;
    let out = run_sb(&mut cpu, &mut bus, sb, &mut left_now, watch);
    (out, cpu, bus, left_now)
}

/// `(exit, target)` of the run and the observable counters after it.
fn seen(r: &Ran) -> Seen {
    let (out, cpu, bus, left) = r;
    (
        (out.exit, out.target),
        (cpu.insns - 100, *left, bus.pc, cpu.r[15], cpu.thumb),
    )
}

/// `add r1,r1,#1` then the loop-back of a 2-instruction iteration (`n_guest` 2).
fn counting_loop() -> Sb {
    trace(
        vec![add(1, 1, PC), control(k::LOOP_BACK, 0, 0, 1, PC + 4)],
        vec![exit(ExitKind::LoopBack, 2, PC + 4, PC)],
        2,
    )
}

suite! {
    #[test]
    fn a_direct_exit_applies_done_last_pc_target_and_the_flags() {
        // mov r0,#5 ; cmp r0,#5 ; (b 0x2000 as an EXIT): done 3, last pc 0x1008, Z and C set
        let sb = trace(
            vec![mov(0, 5, PC), cmp(0, 5, PC + 4), control(k::EXIT, 0, 0, 1, PC + 8)],
            vec![exit(ExitKind::Direct, 3, PC + 8, 0x2000)],
            3,
        );
        let r = go(&sb, 10, false, |_, _| {});
        assert_eq!(seen(&r), ((0, 0x2000), (3, 7, PC + 8, 0x2000, false)));
        let cpu = &r.1;
        assert_eq!((cpu.r[0], cpu.z, cpu.c, cpu.n, cpu.v), (5, true, true, false, false));
    }

    #[test]
    fn a_bx_exit_reads_the_register_and_interworks_on_bit_zero() {
        let sb = trace(
            vec![mov(1, 1, PC), control(k::EXIT_REG, 0, 0, 1, PC + 4)],
            vec![{
                let mut e = exit(ExitKind::Indirect, 2, PC + 4, 0);
                e.reg = 3;
                e
            }],
            2,
        );
        let thumb = go(&sb, 5, false, |c, _| c.r[3] = 0x3001);
        assert_eq!(seen(&thumb), ((0, 0x3000), (2, 3, PC + 4, 0x3000, true)));
        let arm = go(&sb, 5, false, |c, _| c.r[3] = 0x3006);
        assert_eq!(seen(&arm), ((0, 0x3004), (2, 3, PC + 4, 0x3004, false)));
        let back = go(&sb, 5, false, |c, _| (c.r[3], c.thumb) = (0x3000, true));
        assert!(!seen(&back).1 .4, "bit 0 clear leaves Thumb state");
    }

    #[test]
    fn a_mov_pc_exit_masks_the_word_and_never_switches_state() {
        let sb = trace(
            vec![control(k::EXIT_REG_ARM, 0, 0, 1, PC)],
            vec![{
                let mut e = exit(ExitKind::IndirectArm, 1, PC, 0);
                e.reg = 3;
                e
            }],
            1,
        );
        let r = go(&sb, 5, false, |c, _| c.r[3] = 0x3007);
        assert_eq!(seen(&r), ((0, 0x3004), (1, 4, PC, 0x3004, false)));
    }

    #[test]
    fn a_dynamic_exit_takes_the_pc_the_ender_left_and_counts_the_ender() {
        // add r1,r1,#1 ; beq +16 (Z set from the nibble): the ender is counted, taken or not
        let uops = vec![add(1, 1, PC), Uop::generic_end(0x0A00_0002, PC + 4, 0)];
        let sb = trace(uops, vec![exit(ExitKind::Dynamic, 2, PC + 4, 0)], 2);
        let taken = go(&sb, 5, false, |c, _| c.z = true);
        assert_eq!(seen(&taken), ((0, PC + 4 + 16), (2, 3, PC + 4, PC + 4 + 16, false)));
        let not_taken = go(&sb, 5, false, |_, _| {});
        assert_eq!(seen(&not_taken), ((0, PC + 8), (2, 3, PC + 4, PC + 8, false)));
        assert_eq!(taken.1.r[1], 1);
    }

    #[test]
    fn a_fault_inside_a_trace_is_counted_and_left_for_the_driver() {
        let uops = vec![add(1, 1, PC), Uop::generic_end(0xE7F0_00F0, PC + 4, 0)];
        let sb = trace(uops, vec![exit(ExitKind::Dynamic, 2, PC + 4, 0)], 2);
        let mut r = go(&sb, 2, false, |_, _| {});
        assert_eq!(seen(&r), ((0, PC + 8), (2, 0, PC + 4, PC + 8, false)));
        assert!(r.1.fault.take().is_some());
    }

    #[test]
    fn a_guard_leaves_with_the_count_up_to_the_guard_and_the_flags_it_tested() {
        // cmp r0,#0 ; beq -> exit 1 ; add r2,r2,#1 ; exit 0
        let sb = trace(
            vec![
                cmp(0, 0, PC),
                control(k::GUARD, 0, 1, 1, PC + 4),
                add(2, 1, PC + 8),
                control(k::EXIT, 0, 0, 1, PC + 12),
            ],
            vec![
                exit(ExitKind::Direct, 4, PC + 12, 0x2000),
                exit(ExitKind::Direct, 2, PC + 4, 0x3000),
            ],
            4,
        );
        let guard_taken = go(&sb, 9, false, |c, _| c.r[0] = 0);
        assert_eq!(seen(&guard_taken), ((1, 0x3000), (2, 7, PC + 4, 0x3000, false)));
        assert_eq!((guard_taken.1.r[2], guard_taken.1.z), (0, true));
        let fall = go(&sb, 9, false, |c, _| c.r[0] = 3);
        assert_eq!(seen(&fall), ((0, 0x2000), (4, 5, PC + 12, 0x2000, false)));
        assert_eq!((fall.1.r[2], fall.1.z), (1, false));
    }

    #[test]
    fn an_inlined_return_leaves_through_the_register_exit_when_lr_is_not_the_return_address() {
        let mut g = control(k::GUARD_REG_EQ, 0, 1, 1, PC);
        (g.rn, g.imm) = (14, 0x4004);
        let sb = trace(
            vec![g, add(0, 1, PC + 4), control(k::EXIT, 0, 0, 1, PC + 8)],
            vec![exit(ExitKind::Direct, 3, PC + 8, 0x2000), {
                let mut e = exit(ExitKind::Indirect, 1, PC, 0);
                e.reg = 14;
                e
            }],
            3,
        );
        let ok = go(&sb, 5, false, |c, _| c.r[14] = 0x4004);
        assert_eq!(seen(&ok), ((0, 0x2000), (3, 2, PC + 8, 0x2000, false)));
        let off = go(&sb, 5, false, |c, _| c.r[14] = 0x5001);
        assert_eq!(seen(&off), ((1, 0x5000), (1, 4, PC, 0x5000, true)));
        assert_eq!(off.1.r[0], 0, "nothing after the guard ran");
    }

    #[test]
    fn a_predicated_body_is_skipped_but_the_instructions_still_retire() {
        // cmp r0,#0 ; PRED ne skip 2 ; add r1 ; add r2 ; exit (4 instructions)
        let mut pred = Uop::new(k::PRED, PC + 4);
        (pred.cond, pred.imm, pred.n) = (1, 2, 0);
        let sb = trace(
            vec![cmp(0, 0, PC), pred, add(1, 1, PC + 4), add(2, 1, PC + 8), control(k::EXIT, 0, 0, 1, PC + 12)],
            vec![exit(ExitKind::Direct, 4, PC + 12, 0x2000)],
            4,
        );
        let skipped = go(&sb, 8, false, |c, _| c.r[0] = 0);
        assert_eq!(seen(&skipped), ((0, 0x2000), (4, 4, PC + 12, 0x2000, false)));
        assert_eq!((skipped.1.r[1], skipped.1.r[2]), (0, 0));
        let run = go(&sb, 8, false, |c, _| c.r[0] = 9);
        assert_eq!((run.1.r[1], run.1.r[2], seen(&run).1.0), (1, 1, 4));
    }

    #[test]
    fn a_followed_branch_is_a_no_op_that_counts_one_instruction() {
        // MOV_REG r0,r0 (the uop of a followed `b`): no register and no flag changes
        let mut nop = Uop::new(k::MOV_REG, PC);
        (nop.rd, nop.rm) = (0, 0);
        let sb = trace(vec![nop, control(k::EXIT, 0, 0, 0, PC + 4)], vec![exit(ExitKind::Direct, 1, PC, 0x2000)], 1);
        let r = go(&sb, 3, false, |c, _| {
            c.r[0] = 0x1234_5678;
            (c.n, c.c) = (true, true);
        });
        assert_eq!((r.1.r[0], r.1.n, r.1.z, r.1.c, r.1.v), (0x1234_5678, true, false, true, false));
        assert_eq!(seen(&r).1.0, 1);
    }

    #[test]
    fn a_loop_runs_whole_laps_only_and_stops_before_the_budget_would_be_exceeded() {
        let sb = counting_loop();
        // after lap k the loop re-enters iff left >= 2k + 2, so it runs floor(left / 2) laps
        for left in 2..=41u64 {
            let r = go(&sb, left, false, |_, _| {});
            let laps = left / 2;
            assert_eq!(r.1.r[1], laps as u32, "left {left}");
            assert_eq!(seen(&r), ((0, PC), (2 * laps, left - 2 * laps, PC + 4, PC, false)), "left {left}");
        }
    }

    #[test]
    fn a_loop_with_a_side_exit_counts_the_laps_before_it_plus_the_path_to_the_guard() {
        // subs r0,r0,#1 ; beq out (exit 1, 2 instructions) ; loop back (exit 0, 2 instructions)
        let mut subs = Uop::new(k::SUB_IMM, PC);
        (subs.rd, subs.rn, subs.imm) = (0, 0, 1);
        let sb = trace(
            vec![subs, cmp(0, 0, PC), control(k::GUARD, 0, 1, 1, PC + 4), control(k::LOOP_BACK, 0, 0, 0, PC + 4)],
            vec![exit(ExitKind::LoopBack, 3, PC + 4, PC), exit(ExitKind::Direct, 3, PC + 4, 0x2000)],
            3,
        );
        // r0 = 3: laps 1 and 2 loop, lap 3 reaches zero: 2 * 3 + 3 = 9 instructions
        let out = go(&sb, 50, false, |c, _| c.r[0] = 3);
        assert_eq!(seen(&out), ((1, 0x2000), (9, 41, PC + 4, 0x2000, false)));
        assert_eq!(out.1.r[0], 0);
        // a budget of 7 stops after lap 2 (acc 6, 6 + 3 > 7): the loop-back exit, r0 = 1
        let short = go(&sb, 7, false, |c, _| c.r[0] = 3);
        assert_eq!(seen(&short), ((0, PC), (6, 1, PC + 4, PC, false)));
        assert_eq!(short.1.r[0], 1);
    }

    #[test]
    fn the_watch_request_returns_at_the_loop_head_after_one_lap() {
        let sb = counting_loop();
        let watched = go(&sb, 100, true, |_, _| {});
        assert_eq!(seen(&watched), ((0, PC), (2, 98, PC + 4, PC, false)));
        assert_eq!(watched.1.r[1], 1, "one lap, the budget is not the reason");
        // a trace without a loop is not affected
        let straight = trace(vec![add(1, 1, PC), control(k::EXIT, 0, 0, 1, PC + 4)], vec![exit(ExitKind::Direct, 2, PC + 4, 0x2000)], 2);
        assert_eq!(seen(&go(&straight, 5, true, |_, _| {})), seen(&go(&straight, 5, false, |_, _| {})));
    }

    #[test]
    fn a_store_into_decoded_code_leaves_through_the_early_exit_before_the_next_uop() {
        // str r1,[r2] writes the page of the trace (marked as code); mov r0,#99 must not run
        let mut st = Uop::new(k::STR_IMM, PC);
        (st.rd, st.rn, st.rs, st.cond) = (1, 2, F_U | F_P, 1);
        let sb = trace(
            vec![st, mov(0, 99, PC + 4), control(k::EXIT, 0, 0, 1, PC + 8)],
            vec![exit(ExitKind::Direct, 3, PC + 8, 0x2000), exit(ExitKind::Stale, 1, PC, PC + 4)],
            3,
        );
        let r = go(&sb, 10, false, |c, b| {
            b.mem.write32(PC, 0);
            b.mem.mark_code(PC);
            c.r[2] = 0x1010;
        });
        assert_eq!(seen(&r), ((1, PC + 4), (1, 9, PC, PC + 4, false)));
        assert_eq!((r.1.r[0], r.2.mem.stale_code.clone()), (0, vec![1]), "the driver drains it");
        // the same store to a data page runs the whole trace
        let r = go(&sb, 10, false, |c, b| {
            b.mem.write32(0x9000_0000, 0);
            c.r[2] = 0x9000_0000;
        });
        assert_eq!((seen(&r).0, r.1.r[0]), ((0, 0x2000), 99));
    }

    #[test]
    fn a_block_store_into_decoded_code_ends_the_trace_too() {
        let mut stm = Uop::new(k::STM, PC);
        (stm.rn, stm.rs, stm.sh, stm.imm, stm.cond) = (2, F_U, 2, 0b1100, 1);
        let sb = trace(
            vec![stm, mov(0, 99, PC + 4), control(k::EXIT, 0, 0, 1, PC + 8)],
            vec![exit(ExitKind::Direct, 3, PC + 8, 0x2000), exit(ExitKind::Stale, 1, PC, PC + 4)],
            3,
        );
        let r = go(&sb, 10, false, |c, b| {
            b.mem.write32(PC, 0);
            b.mem.mark_code(PC);
            c.r[2] = 0x1010;
        });
        assert_eq!((seen(&r).0, r.1.r[0]), ((1, PC + 4), 0));
    }

    #[test]
    fn a_fused_uop_equals_its_unfused_expansion_on_random_states() {
        let mut rng = Rng::new(0xE0E0);
        let (mut fused_runs, mut guards_taken, mut loads) = (0, 0, 0);
        for i in 0..3000u32 {
            let (a, b) = random_pair(&mut rng, i, PC);
            let Some(fused) = fuse_pair(&a, &b) else { continue };
            let [ea, eb] = expand(&fused).unwrap();
            let exits = vec![
                exit(ExitKind::Direct, 2, PC + 4, 0x2000),
                exit(ExitKind::Direct, 0, 0, 0),
                exit(ExitKind::Direct, 0, 0, 0),
                exit(ExitKind::Direct, 2, PC + 4, 0x3000),
            ];
            let end = control(k::EXIT, 0, 0, 0, PC + 4);
            let one = trace(vec![fused, end], exits.clone(), 2);
            let two = trace(vec![ea, eb, end], exits, 2);
            let mut cpu = random_cpu(&mut rng, PC);
            // loads and stores stay on addresses the log bus handles; the flags come from the cpu
            cpu.r[(fused.rm.min(14)) as usize] = rng.edge();
            cpu.r[fused.rn.min(14) as usize] = rng.below(0x40) << 2 | 0x9000_0000;
            let (mut want_cpu, mut want_bus) = (cpu.clone(), LogBus::new(5));
            let (mut got_cpu, mut got_bus) = (cpu.clone(), LogBus::new(5));
            let (mut want_left, mut got_left) = (9u64, 9u64);
            let want = run_sb(&mut want_cpu, &mut want_bus, &two, &mut want_left, false);
            let got = run_sb(&mut got_cpu, &mut got_bus, &one, &mut got_left, false);
            assert_eq!(got, want, "{fused:?}");
            assert_eq!(CpuSnap::of(&got_cpu, None).diff(&CpuSnap::of(&want_cpu, None)), None, "{fused:?}");
            assert_eq!((got_left, got_bus.pc, got_bus.diff(&want_bus)), (want_left, want_bus.pc, None), "{fused:?}");
            fused_runs += 1;
            guards_taken += usize::from(got.exit == 3);
            loads += want_bus.loads();
        }
        assert!(fused_runs > 1500, "{fused_runs} fused cases");
        assert!(guards_taken > 100 && loads > 600, "{guards_taken} guards, {loads} loads");
    }
}
