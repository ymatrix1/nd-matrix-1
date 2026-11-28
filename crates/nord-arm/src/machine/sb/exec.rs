//! OWNER: W2-XCORE. The tier-2 trace executor: `run_sb` runs one trace from its first uop to an
//! exit and applies the exit.
//!
//! Spec (design note 4.3, 4.4, WP W2-XCORE): one `loop` over the uops with the registers
//! (`cpu.r`), the flag nibble `f` (I4) and the index `i` in locals, dispatching on the static
//! registry `k::family`:
//!
//! | Family | Arm |
//! |---|---|
//! | `Dp` (`0..=111`) | `exec_alu::exec_dp` on `cpu.r` and `f` (`108..=111`: `MUL`, `MULS`, `MLA`, `MLAS`) |
//! | `Mem` (`112..=128`) | `exec_mem::exec_mem`; after a store `bus.stale()` ends the trace through the uop's early exit (I6) |
//! | `Flow`, `Fused` | `exec_flow::exec_flow` (`PRED`, `GUARD*`, `LOOP_BACK`, `EXIT*`, `GENERIC*`, `MRS`, `MSR_C`, the fused kinds) |
//! | anything else | `unreachable!`: a guest cannot produce it, only a lowerer bug can |
//!
//! `kinds.rs` runs one uop of every defined id (no arm may reach `unreachable!`) and checks that
//! every other id does. The long multiplies and the halfword multiplies have no kind id and stay
//! `GENERIC`.
//!
//! # Applying the exit ([`finish`])
//!
//! The exit is applied here, by whoever runs the trace (`uop.rs` conventions): flags written back
//! (`cpu.n/z/c/v`), `cpu.insns += acc + done`, `*left -= acc + done` (`acc` = the instructions of
//! the completed loop iterations, `exec_flow::Lap`), `cpu.r[15] = target`, `bus.pc = last_pc`. The
//! target of an `Indirect` exit switches `cpu.thumb` on bit 0 (`bx`), a `Dynamic` exit takes the
//! `r15` the `GENERIC_END` instruction left. A fault stays in `cpu.fault` for the driver, which
//! takes it after every run and drains `Mem::stale_code` too (design note 4.1).
//!
//! `watch` (idle v2, `SbState::watch.armed`): `LOOP_BACK` does not re-enter, it leaves through its
//! exit, whose target is the loop head: the dispatch point the loop-watch compares at. The driver
//! passed `watch = true` and treats the exit as the `Watch` return (`run.rs`, `loopwatch.rs`); a
//! trace without a loop is unaffected.
//!
//! # Mutation sites (L5, ids `450..=499` of `mutants.rs`)
//!
//! [`MUTANTS`] lists the sites of this file; those of `exec_flow.rs` and its children are
//! `exec_flow::MUTANTS` (`400..=449`). `kills.rs` proves each one is killed by this package's
//! own suites.

use super::backend::RunOut;
use super::bus::SbBus;
use super::exec_alu::exec_dp;
use super::exec_flow::{exec_flow, mutant, Lap, Step};
use super::exec_mem::exec_mem;
use super::uop::{k, pack_flags, unpack_flags, Exit, ExitKind, Sb};
use crate::cpu::Cpu;

#[cfg(test)]
mod exits;
#[cfg(test)]
mod kills;
#[cfg(test)]
mod kinds;
#[cfg(test)]
mod stores;

/// `(id, name)` of the mutation sites of `exec.rs` (block `450..=499`).
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (450, "run_sb: no stale exit after a store"),
    (451, "finish: the loop accumulator is not counted"),
    (452, "finish: the flags are not written back"),
    (453, "finish: bus.pc is not set"),
    (454, "finish: an indirect exit does not interwork"),
    (455, "finish: the budget is not decremented"),
    (456, "finish: cpu.r[15] is not set"),
    (457, "finish: the exit counts one instruction too few"),
    (
        458,
        "run_sb: only STR_IMM and STM end the trace after a store",
    ),
];

/// Run trace `sb` from its first uop. The caller guarantees I7 (`*left >= sb.n_guest`, no page
/// of the trace holds `until`/`stop`). With `watch` a loop is left at its head instead of being
/// re-entered (module docs).
pub fn run_sb<B: SbBus>(
    cpu: &mut Cpu,
    bus: &mut B,
    sb: &Sb,
    left: &mut u64,
    watch: bool,
) -> RunOut {
    let mut f = pack_flags(cpu);
    let mut lap = Lap::start(*left, watch);
    let mut i = 0usize;
    let exit = loop {
        let Some(u) = sb.uops.get(i) else {
            unreachable!(
                "a trace ends with a terminal uop (uop {i} of {})",
                sb.uops.len()
            );
        };
        match k::family(u.kind) {
            k::Family::Dp => {
                exec_dp(&mut cpu.r, &mut f, u);
                i += 1;
            }
            k::Family::Mem => {
                exec_mem(&mut cpu.r, bus, u);
                let ends = if mutant(458) {
                    matches!(u.kind, k::STR_IMM | k::STM)
                } else {
                    k::is_store(u.kind)
                };
                if ends && bus.stale() && !mutant(450) {
                    break u.exit_idx();
                }
                i += 1;
            }
            k::Family::Flow | k::Family::Fused => {
                match exec_flow(cpu, bus, sb, i, &mut f, &mut lap) {
                    Step::Next(n) => i = n,
                    Step::Exit(e) => break e,
                }
            }
            k::Family::Invalid => unreachable!("unknown uop kind {}", u.kind),
        }
    };
    finish(cpu, bus, sb, exit, f, lap.acc, left)
}

/// The pc after leaving through `e` (also switches to Thumb for a `bx`-style exit).
fn exit_target(cpu: &mut Cpu, e: &Exit) -> u32 {
    match e.kind {
        ExitKind::Dynamic => cpu.r[15],
        ExitKind::Indirect => {
            let v = cpu.r[e.reg as usize];
            if !mutant(454) {
                cpu.thumb = v & 1 != 0;
            }
            if cpu.thumb {
                v & !1
            } else {
                v & !3
            }
        }
        ExitKind::IndirectArm => cpu.r[e.reg as usize] & !3,
        ExitKind::Direct
        | ExitKind::Fault
        | ExitKind::Stale
        | ExitKind::LoopBack
        | ExitKind::Watch => e.target,
    }
}

/// Apply exit `exit` of `sb` after `acc` instructions of completed loop iterations (see the
/// module docs) and report it.
fn finish<B: SbBus>(
    cpu: &mut Cpu,
    bus: &mut B,
    sb: &Sb,
    exit: usize,
    f: u8,
    acc: u64,
    left: &mut u64,
) -> RunOut {
    let e = &sb.exits[exit];
    if !mutant(452) {
        unpack_flags(cpu, f);
    }
    let iterations = if mutant(451) { 0 } else { acc };
    let done = (iterations + u64::from(e.done)).saturating_sub(u64::from(mutant(457)));
    debug_assert!(
        *left >= done,
        "I7: trace entered with too few instructions left"
    );
    cpu.insns += done;
    if !mutant(455) {
        *left = left.saturating_sub(done);
    }
    let target = exit_target(cpu, e);
    if !mutant(456) {
        cpu.r[15] = target;
    }
    if !mutant(453) {
        bus.set_pc(e.last_pc);
    }
    RunOut {
        exit: exit as u16,
        target,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::uop::Uop;
    use crate::machine::SysBus;

    fn trace(uops: Vec<Uop>, exits: Vec<Exit>, n_guest: u16) -> Sb {
        Sb::from_parts(0x1000, n_guest, uops, exits, vec![(0x1000, 0)]).unwrap()
    }

    #[test]
    fn every_id_is_either_registered_to_one_family_or_unreachable() {
        let mut per_family = [0usize; 5];
        for id in 0..=255u8 {
            let slot = match k::family(id) {
                k::Family::Dp => 0,
                k::Family::Mem => 1,
                k::Family::Flow => 2,
                k::Family::Fused => 3,
                k::Family::Invalid => 4,
            };
            per_family[slot] += 1;
        }
        assert_eq!(per_family, [112, 17, 11, 5, 256 - k::DEFINED]);
    }

    #[test]
    fn straight_line_generic_uops_end_in_a_direct_exit_with_the_static_counts() {
        // mov r0,#5 ; add r0,r0,#2 ; exit (done 2, last pc 0x1004, target 0x1008)
        let sb = trace(
            vec![
                Uop::generic(0xE3A0_0005, 0x1000, 1),
                Uop::generic(0xE280_0002, 0x1004, 1),
                Uop::exit(0, 0x1004),
            ],
            vec![
                Exit::new(ExitKind::Direct, 2, 0x1004, 0x1008),
                Exit::new(ExitKind::Stale, 0, 0, 0),
            ],
            2,
        );
        let (mut cpu, mut bus, mut left) = (Cpu::new(), SysBus::new(), 10u64);
        cpu.insns = 100;
        let out = run_sb(&mut cpu, &mut bus, &sb, &mut left, false);
        assert_eq!(
            out,
            RunOut {
                exit: 0,
                target: 0x1008
            }
        );
        assert_eq!(
            (cpu.r[0], cpu.r[15], cpu.insns, left, bus.pc),
            (7, 0x1008, 102, 8, 0x1004)
        );
    }

    #[test]
    fn exit_targets_follow_the_exit_kind() {
        let mut cpu = Cpu::new();
        cpu.r[3] = 0x2001;
        cpu.r[15] = 0x3000;
        let mut e = Exit::new(ExitKind::Direct, 1, 0, 0x1234);
        assert_eq!(exit_target(&mut cpu, &e), 0x1234);
        e.kind = ExitKind::Dynamic;
        assert_eq!(exit_target(&mut cpu, &e), 0x3000);
        e.kind = ExitKind::IndirectArm;
        e.reg = 3;
        assert_eq!((exit_target(&mut cpu, &e), cpu.thumb), (0x2000, false));
        e.kind = ExitKind::Indirect;
        assert_eq!((exit_target(&mut cpu, &e), cpu.thumb), (0x2000, true));
        cpu.r[3] = 0x2007;
        cpu.thumb = true;
        assert_eq!((exit_target(&mut cpu, &e), cpu.thumb), (0x2006, true));
        cpu.r[3] = 0x2006;
        assert_eq!((exit_target(&mut cpu, &e), cpu.thumb), (0x2004, false));
    }

    #[test]
    fn the_mutation_sites_are_off_outside_a_mutant_run_and_inside_the_block() {
        assert!(MUTANTS.iter().all(|(id, _)| !mutant(*id)));
        assert!(MUTANTS.iter().all(|(id, _)| (450..=499).contains(id)));
        let mut ids: Vec<u32> = MUTANTS.iter().map(|m| m.0).collect();
        ids.dedup();
        assert_eq!(ids.len(), MUTANTS.len());
    }
}
