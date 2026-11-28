//! OWNER: W2-XCORE. Executor arms of the control uops (skeleton: `GENERIC`, `GENERIC_END` and
//! `EXIT` only).
//!
//! Spec (design note 4.3, WP W2-XCORE): `PRED` (skip `imm` uops when the condition fails),
//! `GUARD`, `GUARD_REG_EQ`, `LOOP_BACK` (local accumulator, re-enter while `left >= acc +
//! n_guest`), `EXIT`/`EXIT_REG` (interwork)/`EXIT_REG_ARM`, `GENERIC` (a non-ender through
//! `exec_pure`; a store may leave `stale_code` non-empty, then leave through the early exit),
//! `GENERIC_END` (an ender through `exec_decoded` with the nibble spilled and reloaded; a fault
//! stays in `cpu.fault` and the uop leaves through its own exit), `MRS`/`MSR_C` (`cpsr_c` only, T
//! never written, `mode_changes` counted), the fused arms (each equal to its unfused expansion),
//! and the `Watch` exit. An unknown kind is `unreachable!`.
//!
//! Registered statically in `exec::run_sb` (families `Flow` and `Fused`); `Step` and the
//! signature below belong to this work package and `exec.rs` (both owned by W2-XCORE).

use super::bus::SbBus;
use super::uop::{k, pack_flags, unpack_flags, Sb};
use crate::cpu::predecode::decode_arm;
use crate::cpu::Cpu;

/// What the executor loop does after one control uop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Continue at this uop index.
    Next(usize),
    /// Leave through `sb.exits[exit]`.
    Exit(usize),
}

/// Execute `sb.uops[i]` (a `Flow` or `Fused` kind) with the flag nibble in `f`.
pub fn exec_flow<B: SbBus>(cpu: &mut Cpu, bus: &mut B, sb: &Sb, i: usize, f: &mut u8) -> Step {
    let u = &sb.uops[i];
    match u.kind {
        k::GENERIC => {
            bus.set_pc(u.pc);
            unpack_flags(cpu, *f);
            cpu.exec_pure(bus, decode_arm(u.imm), u.pc);
            *f = pack_flags(cpu);
            if bus.stale() {
                Step::Exit(u.exit_idx())
            } else {
                Step::Next(i + 1)
            }
        }
        k::GENERIC_END => {
            bus.set_pc(u.pc);
            unpack_flags(cpu, *f);
            // a fault stays in `cpu.fault`; the driver takes it after the run
            cpu.exec_decoded(bus, decode_arm(u.imm), u.pc);
            *f = pack_flags(cpu);
            Step::Exit(u.exit_idx())
        }
        k::EXIT => Step::Exit(u.exit_idx()),
        kind => unreachable!("uop kind {kind} has no executor arm yet (OWNER W2-XCORE)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::uop::{Exit, ExitKind, Uop};
    use crate::machine::SysBus;

    fn one(uops: Vec<Uop>) -> Sb {
        let exits = vec![Exit::new(ExitKind::Direct, 1, 0x1000, 0x1004)];
        Sb::from_parts(0x1000, 1, uops, exits, vec![(0x1000, 0)]).unwrap()
    }

    #[test]
    fn generic_runs_a_non_ender_and_continues_with_the_flags_in_the_nibble() {
        let sb = one(vec![Uop::generic(0xE250_0001, 0x1000, 0)]); // subs r0,r0,#1
        let (mut cpu, mut bus, mut f) = (Cpu::new(), SysBus::new(), 0u8);
        cpu.r[0] = 1;
        assert_eq!(exec_flow(&mut cpu, &mut bus, &sb, 0, &mut f), Step::Next(1));
        assert_eq!((cpu.r[0], f), (0, 0b0110), "result 0: Z and C set");
        assert_eq!(bus.pc, 0x1000);
    }

    #[test]
    fn generic_end_runs_an_ender_and_leaves_through_its_exit() {
        let sb = one(vec![Uop::generic_end(0xEAFF_FFFE, 0x1000, 0)]); // b .
        let (mut cpu, mut bus, mut f) = (Cpu::new(), SysBus::new(), 0u8);
        assert_eq!(exec_flow(&mut cpu, &mut bus, &sb, 0, &mut f), Step::Exit(0));
        assert_eq!(
            cpu.r[15], 0x1000,
            "the branch target is in r15 (a Dynamic exit)"
        );
    }

    #[test]
    fn a_faulting_ender_leaves_the_fault_for_the_driver() {
        let sb = one(vec![Uop::generic_end(0xE7F0_00F0, 0x1000, 0)]); // undefined
        let (mut cpu, mut bus, mut f) = (Cpu::new(), SysBus::new(), 0u8);
        assert_eq!(exec_flow(&mut cpu, &mut bus, &sb, 0, &mut f), Step::Exit(0));
        assert!(cpu.fault.is_some());
    }

    #[test]
    fn exit_names_the_exit_of_the_uop() {
        let sb = one(vec![Uop::exit(0, 0x1000)]);
        let (mut cpu, mut bus, mut f) = (Cpu::new(), SysBus::new(), 0u8);
        assert_eq!(exec_flow(&mut cpu, &mut bus, &sb, 0, &mut f), Step::Exit(0));
    }
}
