//! OWNER: W1-FLOW (test support, `cfg(test)`). A reference executor for the traces `build`
//! makes and a driver that runs a machine on traces only, so the builder is tested against the
//! interpreter before the real executor (`exec.rs`, W2-XCORE) exists.
//!
//! The executor knows every control uop the builder emits (`PRED`, `GUARD`, `GUARD_REG_EQ`,
//! `LOOP_BACK`, `EXIT*`, `GENERIC*`) and the `MOV_IMM` of a `bl` link, and hands the data-processing and memory families to the
//! frozen arms `exec_alu::exec_dp` and `exec_mem::exec_mem`, so the same tests keep working once
//! the lowerers of W1-DP and W1-MEM land. It applies an exit as `exec::finish` does and reads the
//! `LOOP_BACK` contract as design note 4.3 gives it: the exit's `done` is one iteration, the
//! executor accumulates and re-enters while `left >= acc + n_guest`.
//!
//! [`drive`] is `Machine::run` on that executor: at every pc it builds a trace, enters it when
//! `left >= n_guest` (I7) and otherwise steps one instruction. [`Tally`] records what ran, so a
//! differential test can prove it exercised something.

use super::super::bus::SbBus;
use super::super::exec_alu::exec_dp;
use super::super::exec_mem::exec_mem;
use super::super::uop::{holds, k, pack_flags, unpack_flags, ExitKind, Sb};
use super::{build_flipped, BuildOpts};
use crate::cpu::predecode::decode_arm;
use crate::cpu::Cpu;
use crate::machine::{Machine, Stop};
use std::collections::BTreeMap;

/// Declare the test functions of a suite (their attributes are passed through) and `run_all`,
/// which calls them in order: the mutation run (`lower_flow/kills.rs`) replays whole suites under
/// a mutant, and one failing assertion is a kill.
macro_rules! suite {
    ($($(#[$meta:meta])* fn $name:ident() $body:block)*) => {
        $($(#[$meta])* fn $name() $body)*

        /// Every test of the suite, in order.
        pub(in crate::machine::sb) fn run_all() {
            $($name();)*
        }
    };
}
pub(in crate::machine::sb) use suite;

/// What one trace run did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ran {
    /// Index of the exit taken.
    pub exit: usize,
    /// Kind of the uop that took it.
    pub via: u8,
    /// Completed loop iterations before the exit.
    pub laps: u64,
}

/// Run `sb` from its first uop and apply the exit. The caller guarantees `*left >= sb.n_guest`.
pub(super) fn run_trace<B: SbBus>(cpu: &mut Cpu, bus: &mut B, sb: &Sb, left: &mut u64) -> Ran {
    let mut f = pack_flags(cpu);
    let (mut i, mut acc, mut laps) = (0usize, 0u64, 0u64);
    let (exit, via) = loop {
        let u = &sb.uops[i];
        match u.kind {
            k::PRED => {
                i += if holds(u.cond, f) {
                    1
                } else {
                    1 + u.imm as usize
                };
            }
            k::GUARD => {
                if holds(u.cond, f) {
                    break (u.exit_idx(), u.kind);
                }
                i += 1;
            }
            k::GUARD_REG_EQ => {
                if cpu.r[u.rn as usize] != u.imm {
                    break (u.exit_idx(), u.kind);
                }
                i += 1;
            }
            k::LOOP_BACK => {
                let iteration = u64::from(sb.exits[u.exit_idx()].done);
                if *left >= acc + iteration + u64::from(sb.n_guest) {
                    acc += iteration;
                    laps += 1;
                    i = 0;
                } else {
                    break (u.exit_idx(), u.kind);
                }
            }
            k::EXIT | k::EXIT_REG | k::EXIT_REG_ARM => break (u.exit_idx(), u.kind),
            k::GENERIC => {
                bus.set_pc(u.pc);
                unpack_flags(cpu, f);
                cpu.exec_pure(bus, decode_arm(u.imm), u.pc);
                f = pack_flags(cpu);
                if bus.stale() {
                    break (u.exit_idx(), u.kind);
                }
                i += 1;
            }
            k::GENERIC_END => {
                bus.set_pc(u.pc);
                unpack_flags(cpu, f);
                cpu.exec_decoded(bus, decode_arm(u.imm), u.pc);
                f = pack_flags(cpu);
                break (u.exit_idx(), u.kind);
            }
            k::MOV_IMM => {
                // the link of a `bl`; the real `exec_dp` arm (W1-XALU) does the same
                cpu.r[u.rd as usize] = u.imm;
                i += 1;
            }
            _ => match k::family(u.kind) {
                k::Family::Dp => {
                    exec_dp(&mut cpu.r, &mut f, u);
                    i += 1;
                }
                k::Family::Mem => {
                    exec_mem(&mut cpu.r, bus, u);
                    if k::is_store(u.kind) && bus.stale() {
                        break (u.exit_idx(), u.kind);
                    }
                    i += 1;
                }
                _ => panic!("the test executor does not know uop kind {}", u.kind),
            },
        }
    };
    let e = &sb.exits[exit];
    unpack_flags(cpu, f);
    let done = acc + u64::from(e.done);
    assert!(
        *left >= done,
        "I7: a trace ran with too few instructions left"
    );
    cpu.insns += done;
    *left -= done;
    cpu.r[15] = match e.kind {
        ExitKind::Dynamic => cpu.r[15],
        ExitKind::Indirect => {
            let v = cpu.r[e.reg as usize];
            cpu.thumb = v & 1 != 0;
            if cpu.thumb {
                v & !1
            } else {
                v & !3
            }
        }
        ExitKind::IndirectArm => cpu.r[e.reg as usize] & !3,
        _ => e.target,
    };
    bus.set_pc(e.last_pc);
    Ran { exit, via, laps }
}

/// What a [`drive`] run exercised.
#[derive(Clone, Debug, Default)]
pub(super) struct Tally {
    /// Trace runs, and the instructions they retired.
    pub traces: u64,
    pub insns: u64,
    /// Instructions run one at a time (no trace, Thumb, a budget below `n_guest`).
    pub steps: u64,
    /// Loop iterations beyond the first.
    pub laps: u64,
    /// Exits taken by the kind of the uop that took them, and by exit kind.
    pub via: BTreeMap<u8, u64>,
    pub exits: [u64; ExitKind::COUNT],
    /// Uops of every kind in the traces that ran (counted per run, not per execution).
    pub kinds: BTreeMap<u8, u64>,
}

impl Tally {
    /// Times an exit was taken by a uop of `kind`.
    pub fn took(&self, kind: u8) -> u64 {
        self.via.get(&kind).copied().unwrap_or(0)
    }

    /// Uops of `kind` in the traces that ran.
    pub fn had(&self, kind: u8) -> u64 {
        self.kinds.get(&kind).copied().unwrap_or(0)
    }
}

/// `Machine::run(max, u32::MAX, None)` with traces: build at every pc (`flips` as for
/// `build_flipped`), run when the budget covers the whole trace, step otherwise. Pages of a trace
/// are marked as code and the stale queue is cleared after every run, as the driver does.
pub(super) fn drive(
    m: &mut Machine,
    max: u64,
    opts: &BuildOpts,
    flips: &[u32],
    tally: &mut Tally,
) -> (u64, Stop) {
    let start = m.cpu.insns;
    let limit = start + max;
    let reason = loop {
        if m.cpu.insns >= limit {
            break Stop::Budget;
        }
        m.bus.mem.stale_code.clear(); // traces are built fresh from memory at every pc
        let pc = m.cpu.r[15];
        let left = limit - m.cpu.insns;
        let sb = if m.cpu.thumb {
            None
        } else {
            build_flipped(&mut m.bus.mem, pc, opts, flips)
        };
        match sb {
            Some(sb) if left >= u64::from(sb.n_guest) => {
                for p in &sb.pages[..sb.n_pages as usize] {
                    m.bus.mem.mark_code(p << 12);
                }
                for u in sb.uops.iter() {
                    *tally.kinds.entry(u.kind).or_default() += 1;
                }
                let mut left = left;
                let before = m.cpu.insns;
                let ran = run_trace(&mut m.cpu, &mut m.bus, &sb, &mut left);
                tally.traces += 1;
                tally.insns += m.cpu.insns - before;
                tally.laps += ran.laps;
                *tally.via.entry(ran.via).or_default() += 1;
                tally.exits[sb.exits[ran.exit].kind as usize] += 1;
                m.bus.mem.stale_code.clear();
                if let Some(f) = m.cpu.fault.take() {
                    break Stop::Fault(f);
                }
            }
            _ => match m.run(1, u32::MAX, None) {
                (_, Stop::Budget) => tally.steps += 1,
                (_, other) => break other,
            },
        }
    };
    (m.cpu.insns - start, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::config::SbConfig;
    use crate::machine::sb::testgen::{machine_with, tiny_loop};
    use crate::machine::sb::testkit::snapshot;
    use crate::machine::sb::uop::{Exit, Uop};
    use crate::machine::SysBus;

    fn trace(uops: Vec<Uop>, exits: Vec<Exit>, n_guest: u16) -> Sb {
        Sb::from_parts(0x1000, n_guest, uops, exits, vec![(0x1000, 0)]).unwrap()
    }

    #[test]
    fn pred_skips_its_body_when_the_condition_fails_and_a_guard_leaves_when_it_holds() {
        // pred(eq){mov r0,#1} ; guard(ne) -> exit 0 ; exit 1
        let mut pred = Uop::new(k::PRED, 0x1000);
        pred.cond = 0;
        pred.imm = 1;
        let mut guard = Uop::new(k::GUARD, 0x1004);
        guard.cond = 1;
        let sb = trace(
            vec![
                pred,
                Uop::generic(0xE3A0_0001, 0x1000, 2),
                guard,
                Uop::exit(1, 0x1008),
            ],
            vec![
                Exit::new(ExitKind::Direct, 2, 0x1004, 0x2000),
                Exit::new(ExitKind::Direct, 3, 0x1008, 0x100C),
                Exit::new(ExitKind::Stale, 1, 0x1000, 0x1004),
            ],
            3,
        );
        for (flags_z, r0, exit) in [(false, 0, 0), (true, 1, 1)] {
            let (mut cpu, mut bus, mut left) = (Cpu::new(), SysBus::new(), 10u64);
            cpu.z = flags_z;
            let ran = run_trace(&mut cpu, &mut bus, &sb, &mut left);
            assert_eq!((ran.exit, cpu.r[0]), (exit, r0), "z = {flags_z}");
        }
    }

    #[test]
    fn a_loop_runs_while_the_budget_covers_another_iteration_and_leaves_with_the_total() {
        // add r0,r0,#1 ; loop_back (iteration of 2 instructions, exit done 2)
        let sb = trace(
            vec![Uop::generic(0xE280_0001, 0x1000, 1), {
                let mut l = Uop::new(k::LOOP_BACK, 0x1004);
                l.rs = 0;
                l
            }],
            vec![
                Exit::new(ExitKind::LoopBack, 2, 0x1004, 0x1000),
                Exit::new(ExitKind::Stale, 1, 0x1000, 0x1004),
            ],
            2,
        );
        let (mut cpu, mut bus, mut left) = (Cpu::new(), SysBus::new(), 7u64);
        let ran = run_trace(&mut cpu, &mut bus, &sb, &mut left);
        // 7 left: laps while left >= acc + 2 + 2, so acc reaches 4, then exits with 6 retired
        assert_eq!((ran.laps, cpu.insns, left, cpu.r[0]), (2, 6, 1, 3));
        assert_eq!((cpu.r[15], bus.pc), (0x1000, 0x1004));
    }

    #[test]
    fn the_driver_equals_the_interpreter_on_the_hand_written_loop_and_runs_traces() {
        let words = tiny_loop();
        for budget in [1u64, 7, 40, 100, 333] {
            let (mut a, mut b) = (
                machine_with(&words, 4, false),
                machine_with(&words, 4, false),
            );
            let ra = a.run(budget, u32::MAX, None);
            let mut tally = Tally::default();
            let rb = drive(
                &mut b,
                budget,
                &BuildOpts::from(&SbConfig::on()),
                &[],
                &mut tally,
            );
            assert_eq!(ra, rb, "budget {budget}");
            assert_eq!(
                snapshot(&a, None).diff(&snapshot(&b, None)),
                None,
                "budget {budget}"
            );
            if budget >= 100 {
                assert!(
                    tally.traces > 0 && tally.insns > 0,
                    "nothing ran on a trace"
                );
            }
        }
    }
}
