//! Options of the live (windowed) app, as opposed to the deterministic render/self-test paths.
//!
//! The live app plays like the hardware where the hardware default is known, even when the
//! offline render default stays unchanged (D22). Issue #678: with the Synth SUSTPED flag off,
//! CC64 held nothing outside the Pianos.
//!
//! Docs: `docs/specs/midi-and-control.md` (SUSTPED), `docs/research/synth-sustped.md`.

use nord_core::render_opts::FirmwareOptions;

/// Options the windowed app starts with, given the parsed CLI options.
///
/// VERIFIED: the Synth SUSTPED default is on (`Edt_Syn_kSustainEnable` `0xF8`, default 1) and
/// 381 / 383 / 383 of the 384 factory programs store 1 for layers A / B / C, see
/// `docs/research/synth-sustped.md`, checked by `python tools/synth_sustped.py`. The Organ default
/// is off and stays off, so the organ sustains only after Shift + Layer A (SUSTPED).
pub fn live_options(mut opts: FirmwareOptions) -> FirmwareOptions {
    opts.synth_sustain_pedal = true;
    opts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{apply_audio_command, AudioCommand};
    use nord_core::engine::Stage4Engine;

    fn synth_sustped_after(opts: FirmwareOptions) -> [bool; 3] {
        let mut e = Stage4Engine::new(48_000.0);
        let mut vol = 1.0;
        apply_audio_command(AudioCommand::SetFirmwareOptions(opts), &mut e, &mut vol);
        [
            e.pedals.sustped[4],
            e.pedals.sustped[5],
            e.pedals.sustped[6],
        ]
    }

    #[test]
    fn live_app_routes_the_sustain_pedal_to_all_synth_layers() {
        assert_eq!(
            synth_sustped_after(live_options(FirmwareOptions::default())),
            [true; 3]
        );
    }

    #[test]
    fn offline_default_keeps_synth_release_immediate() {
        assert_eq!(synth_sustped_after(FirmwareOptions::default()), [false; 3]);
    }

    #[test]
    fn live_options_change_nothing_else() {
        let live = live_options(FirmwareOptions::default());
        let expect = FirmwareOptions {
            synth_sustain_pedal: true,
            ..FirmwareOptions::default()
        };
        assert_eq!(live, expect);
    }

    #[test]
    fn loaded_program_bit_decides_each_synth_layer() {
        let mut e = Stage4Engine::new(48_000.0);
        let mut vol = 1.0;
        apply_audio_command(
            AudioCommand::SetFirmwareOptions(live_options(FirmwareOptions::default())),
            &mut e,
            &mut vol,
        );
        let mut prog = nord_core::cbin::ProgramDecoded::default();
        prog.global.synth_sustain = [true, false, true];
        apply_audio_command(AudioCommand::LoadProgram(Box::new(prog)), &mut e, &mut vol);
        assert_eq!(e.pedals.sustped[4..7], [true, false, true]);
    }
}
