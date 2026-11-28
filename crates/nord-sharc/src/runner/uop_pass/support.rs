//! Test fixtures shared by the `uop_pass` modules: a straight run of NOPs and the poll program
//! (ten NOPs, then `IF TRUE JUMP CODE`) with its pass configuration.
//!
//! OWNER: W1-RUN. Hand-written parcels, no firmware.
use crate::core::support::core_with;
use crate::core::Core;
use crate::runner::pass::PassConfig;

/// Short-word address of the code of the fixtures (the VISA code window).
pub const CODE: u32 = 0x12_0000;
/// Mailbox word the poll program reads: a scripted device word that always reads 0, so the pass
/// ends whenever the pc is back on the poll instruction.
pub const POLL_TRIGGER: u32 = 0x3000_0000;
/// The poll instruction `IF TRUE JUMP CODE`, after ten NOPs from [`CODE`]. A pass is the jump and
/// the ten NOPs, 11 steps, and ends back on the poll instruction.
pub const POLL_PC: u32 = CODE + 10;

/// 600 NOPs: a pass of 500 steps ends on the step limit without leaving the code.
pub fn core() -> Core {
    core_with(&[0x0001; 600])
}

/// The pass of [`poll_core`].
pub fn poll_cfg() -> PassConfig {
    PassConfig {
        trigger: POLL_TRIGGER,
        idle_pc: POLL_PC,
        max_steps: 100,
        irq: None,
    }
}

/// A core idling at [`POLL_PC`]: ten NOPs from [`CODE`], then `IF TRUE JUMP CODE`.
pub fn poll_core() -> Core {
    let mut c = core_with(&[0x0001; 10]);
    for (k, p) in [0x063e_u16, 0x0012, 0x0000].iter().enumerate() {
        c.dm_write(POLL_PC + k as u32, u32::from(*p)).unwrap();
    }
    c.preset_device(POLL_TRIGGER, 0);
    c.pc = POLL_PC;
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::system::lean_pass;

    #[test]
    fn the_poll_program_ends_every_pass_on_the_poll_instruction() {
        let mut want = poll_core();
        assert_eq!(lean_pass(&mut want, &poll_cfg()), (11, true));
        assert_eq!((want.pc, want.steps), (POLL_PC, 11));
        assert_eq!(lean_pass(&mut want, &poll_cfg()), (11, true), "and again");
    }

    #[test]
    fn the_nop_core_runs_to_the_step_limit() {
        let mut c = core();
        let cfg = PassConfig {
            trigger: 0xB_00E2,
            idle_pc: CODE,
            max_steps: 500,
            irq: None,
        };
        assert_eq!(lean_pass(&mut c, &cfg), (500, false));
    }
}
