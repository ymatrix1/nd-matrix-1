//! The synthetic poll program (no firmware) the tool and the tests use as the wiring check: ten
//! NOPs and a jump, 11 steps per pass.
//!
//! OWNER: W1-RUN. Hand-written parcels.
use crate::runner::live_presets::Preset;

/// Short-word address of the synthetic program (the VISA code window).
pub const SYNTH_CODE: u32 = 0x12_0000;
/// Mailbox address of the synthetic pass: a scripted device word that always reads 0, so a pass
/// ends whenever the pc is back on the poll instruction.
pub const SYNTH_TRIGGER: u32 = 0x3000_0000;
/// The poll instruction of the synthetic program, after its ten NOPs.
pub const SYNTH_POLL_PC: u32 = SYNTH_CODE + 10;
/// Steps of one synthetic pass: the poll jump and the ten NOPs it runs through before landing on
/// the poll instruction again.
pub const SYNTH_PASS_STEPS: u64 = 11;

/// SRAM image of the synthetic program: ten NOPs (`0x0001`) at [`SYNTH_CODE`], then
/// `IF TRUE JUMP SYNTH_CODE` (`0x063e 0x0012 0x0000`) at [`SYNTH_POLL_PC`]. Hand-written parcels,
/// no firmware.
pub fn synthetic_image() -> Vec<u8> {
    let parcels = [
        0x0001u16, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x063e,
        0x0012, 0x0000,
    ];
    let mut image = vec![0u8; SYNTH_CODE as usize * 2 + parcels.len() * 2];
    for (k, p) in parcels.iter().enumerate() {
        let o = (SYNTH_CODE as usize + k) * 2;
        image[o..o + 2].copy_from_slice(&p.to_le_bytes());
    }
    image
}

/// The start-up of the synthetic program: idle on the poll instruction, no boot steps.
pub fn synthetic_preset() -> Preset {
    Preset {
        entry: SYNTH_POLL_PC,
        idle_pc: SYNTH_POLL_PC,
        trigger: SYNTH_TRIGGER,
        boot: 0,
        irq: None,
        ping_pong: None,
        devs: vec![(SYNTH_TRIGGER, 0)],
        flag1: false,
        sy: false,
        smp: false,
        fx: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::live_presets::boot_core;
    use crate::runner::pass_sched::{Engine, PassJob};
    use crate::runner::periph_run::PeriphOpts;

    #[test]
    fn the_synthetic_program_is_a_pass_of_11_steps_that_ends_on_the_poll_instruction() {
        let preset = synthetic_preset();
        let core = boot_core(&synthetic_image(), &preset, &PeriphOpts::default()).unwrap();
        assert_eq!(core.pc, SYNTH_POLL_PC);
        let mut j = PassJob::from_preset("s", core, &preset, vec![], Engine::Interp);
        assert_eq!(j.run_pass(), (SYNTH_PASS_STEPS, true));
        assert_eq!(j.core.pc, SYNTH_POLL_PC);
        assert_eq!(j.run_pass(), (SYNTH_PASS_STEPS, true), "and again");
    }

    #[test]
    fn the_image_holds_the_ten_nops_and_the_jump_at_the_code_address() {
        let image = synthetic_image();
        let at = |k: u32| {
            let o = (SYNTH_CODE + k) as usize * 2;
            u16::from_le_bytes([image[o], image[o + 1]])
        };
        assert!((0..10).all(|k| at(k) == 0x0001));
        assert_eq!((at(10), at(11), at(12)), (0x063e, 0x0012, 0x0000));
        assert_eq!(image.len(), (SYNTH_CODE as usize + 13) * 2);
    }
}
