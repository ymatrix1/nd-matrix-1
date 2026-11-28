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

/// Scratch SRAM word (a normal-word address that is neither code nor the mailbox) that
/// [`mark_wrong_engine`] writes and [`misbehave`] reads.
pub const WRONG_ENGINE_WORD: u32 = 0xA_0100;
/// The value of [`WRONG_ENGINE_WORD`] that marks a core: a value no fixture or image stores there.
const WRONG_ENGINE_MARK: u32 = 0x0BAD_E461;

/// Makes the uop engine wrong on `core`: every engine pass run on it ends with a register
/// flipped, which stands in for an executor bug. The executor of this base is still the skeleton
/// (every handler bails and the interpreter runs the pass), so no real misbehaviour is reachable
/// from the runner, and a test of the checked passes needs a wrong engine to be sure that it can
/// see one. The mark lives in the core's memory, not in a thread-local: it follows the core into
/// the scheduler's worker threads, and the rewind of a checked pass restores it with the rest of
/// the state, so the replaying interpreter never sees a difference it did not cause.
pub fn mark_wrong_engine(core: &mut Core) {
    core.dm_write(WRONG_ENGINE_WORD, WRONG_ENGINE_MARK).unwrap();
}

/// The hook behind [`mark_wrong_engine`]: `uop_body_with` calls it after the engine loop of
/// every pass in a test build (production builds do not have it). Flips bits of `r3` on a marked
/// core, the field `exact_diff` names `r`; any other core is left alone.
pub(super) fn misbehave(core: &mut Core) {
    if core.dm_read(WRONG_ENGINE_WORD) == Ok(WRONG_ENGINE_MARK) {
        core.r[3] ^= 0x10;
    }
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
    fn only_a_marked_core_misbehaves_and_the_mark_is_one_memory_word() {
        let (mut plain, mut marked) = (poll_core(), poll_core());
        let before = marked.r[3];
        mark_wrong_engine(&mut marked);
        misbehave(&mut plain);
        assert_eq!(plain.r[3], before, "an unmarked core is left alone");
        misbehave(&mut marked);
        assert_eq!(marked.r[3], before ^ 0x10, "a marked core is wrong");
        assert_eq!(marked.dm_read(WRONG_ENGINE_WORD), Ok(WRONG_ENGINE_MARK));
        assert_eq!(plain.dm_read(WRONG_ENGINE_WORD), Ok(0));
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
