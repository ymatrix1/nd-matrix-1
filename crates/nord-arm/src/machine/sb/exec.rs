//! OWNER: W2-XCORE. The tier-2 trace executor (skeleton: `GENERIC`, `GENERIC_END` and `EXIT`
//! through the interpreter's own handlers, plus the static dispatch to every family).
//!
//! Spec (design note 4.3, 4.4, WP W2-XCORE): one `loop { match k::family(kind) }` over the uops
//! with the registers (`cpu.r`), the flag nibble `f` and the index `i` in locals; the family
//! arms are `exec_alu::exec_dp`, `exec_mem::exec_mem` and `exec_flow::exec_flow` (registered
//! here statically, their signatures are frozen). After a store `bus.stale()` ends the trace
//! through the uop's early exit (I6). The exit is applied here ([`finish`]): flags written back,
//! `cpu.insns += done`, `*left -= done`, `cpu.r[15] = target`, `bus.pc = last_pc`; a fault stays
//! in `cpu.fault` for the driver. `watch` returns a `Watch` exit at a dispatch point (idle v2).
//! An unknown kind is `unreachable!`; a test enumerates all 256 ids against the registry.

use super::backend::RunOut;
use super::bus::SbBus;
use super::exec_alu::exec_dp;
use super::exec_flow::{exec_flow, Step};
use super::exec_mem::exec_mem;
use super::uop::{k, pack_flags, unpack_flags, Exit, ExitKind, Sb};
use crate::cpu::Cpu;

/// Run trace `sb` from its first uop. The caller guarantees I7 (`*left >= sb.n_guest`, no page
/// of the trace holds `until`/`stop`). The skeleton ignores `watch`.
pub fn run_sb<B: SbBus>(
    cpu: &mut Cpu,
    bus: &mut B,
    sb: &Sb,
    left: &mut u64,
    watch: bool,
) -> RunOut {
    let _ = watch;
    let mut f = pack_flags(cpu);
    let mut i = 0usize;
    let exit = loop {
        let u = &sb.uops[i];
        match k::family(u.kind) {
            k::Family::Dp => {
                exec_dp(&mut cpu.r, &mut f, u);
                i += 1;
            }
            k::Family::Mem => {
                exec_mem(&mut cpu.r, bus, u);
                if k::is_store(u.kind) && bus.stale() {
                    break u.exit_idx();
                }
                i += 1;
            }
            k::Family::Flow | k::Family::Fused => match exec_flow(cpu, bus, sb, i, &mut f) {
                Step::Next(n) => i = n,
                Step::Exit(e) => break e,
            },
            k::Family::Invalid => unreachable!("unknown uop kind {}", u.kind),
        }
    };
    finish(cpu, bus, sb, exit, f, left)
}

/// The pc after leaving through `e` (also switches to Thumb for a `bx`-style exit).
fn exit_target(cpu: &mut Cpu, e: &Exit) -> u32 {
    match e.kind {
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
        ExitKind::Direct
        | ExitKind::Fault
        | ExitKind::Stale
        | ExitKind::LoopBack
        | ExitKind::Watch => e.target,
    }
}

/// Apply exit `exit` of `sb` (see the module docs) and report it.
fn finish<B: SbBus>(
    cpu: &mut Cpu,
    bus: &mut B,
    sb: &Sb,
    exit: usize,
    f: u8,
    left: &mut u64,
) -> RunOut {
    let e = &sb.exits[exit];
    unpack_flags(cpu, f);
    let done = e.done as u64;
    debug_assert!(
        *left >= done,
        "I7: trace entered with too few instructions left"
    );
    cpu.insns += done;
    *left = left.saturating_sub(done);
    let target = exit_target(cpu, e);
    cpu.r[15] = target;
    bus.set_pc(e.last_pc);
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
        assert_eq!(per_family, [108, 17, 11, 5, 256 - k::DEFINED]);
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
    fn a_store_into_decoded_code_leaves_through_the_early_exit_before_the_next_uop() {
        // str r1,[r2] writes the page of the trace (marked as code) ; mov r0,#99 must not run
        let sb = trace(
            vec![
                Uop::generic(0xE582_1000, 0x1000, 1),
                Uop::generic(0xE3A0_0063, 0x1004, 1),
                Uop::exit(0, 0x1004),
            ],
            vec![
                Exit::new(ExitKind::Direct, 2, 0x1004, 0x1008),
                Exit::new(ExitKind::Stale, 1, 0x1000, 0x1004),
            ],
            2,
        );
        let (mut cpu, mut bus, mut left) = (Cpu::new(), SysBus::new(), 10u64);
        bus.mem.write32(0x1000, 0);
        bus.mem.mark_code(0x1000);
        cpu.r[2] = 0x1010;
        let out = run_sb(&mut cpu, &mut bus, &sb, &mut left, false);
        assert_eq!(
            out,
            RunOut {
                exit: 1,
                target: 0x1004
            }
        );
        assert_eq!(
            (cpu.r[0], cpu.r[15], cpu.insns, left, bus.pc),
            (0, 0x1004, 1, 9, 0x1000)
        );
        assert_eq!(bus.mem.stale_code, vec![1], "the driver drains it");
    }

    #[test]
    fn a_faulting_ender_counts_the_instruction_and_leaves_the_fault_in_the_cpu() {
        let sb = trace(
            vec![Uop::generic_end(0xE7F0_00F0, 0x1000, 0)],
            vec![Exit::new(ExitKind::Dynamic, 1, 0x1000, 0)],
            1,
        );
        let (mut cpu, mut bus, mut left) = (Cpu::new(), SysBus::new(), 1u64);
        run_sb(&mut cpu, &mut bus, &sb, &mut left, false);
        assert!(cpu.fault.take().is_some());
        assert_eq!((cpu.insns, left, cpu.r[15]), (1, 0, 0x1004));
    }

    #[test]
    fn the_flags_survive_the_nibble_round_trip_through_generic_uops() {
        // cmp r0,#0 sets Z; the exit writes the nibble back
        let sb = trace(
            vec![Uop::generic(0xE350_0000, 0x1000, 1), Uop::exit(0, 0x1000)],
            vec![
                Exit::new(ExitKind::Direct, 1, 0x1000, 0x1004),
                Exit::new(ExitKind::Stale, 0, 0, 0),
            ],
            1,
        );
        let (mut cpu, mut bus, mut left) = (Cpu::new(), SysBus::new(), 1u64);
        run_sb(&mut cpu, &mut bus, &sb, &mut left, false);
        assert!(cpu.z && cpu.c && !cpu.n && !cpu.v);
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
}
