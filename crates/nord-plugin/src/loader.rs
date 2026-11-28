//! The load path from the editor (GUI thread) to the engine (audio thread), D218.
//!
//! Programs, piano and sample banks and firmware options are **built on the GUI side** (the
//! library worker threads decode `.ns4b` / `.ns4p` / `.npno` / `.nsmp4`; the OS update's WAVE tables
//! are installed once per process off-thread) and handed to `process()` as ready values through a
//! bounded array queue: `try_send` / `try_recv` neither lock nor allocate
//! (`.agents/rules/dsp-and-realtime.md`). The audio thread only moves them into the engine
//! (`apply_program` and `replace_sound` are in-place stores, the same calls the standalone callback
//! makes) and **never frees**: the values it replaces go to a reaper thread through the
//! [`Bin`](nord_gui::audio::garbage::Bin) of the standalone app, or leak when that queue is full.
//!
//! [`Loaded`] keeps what was loaded so that `initialize` / `reset`, which rebuild the engine, put it
//! back (a host reset must not drop the program).

use crate::params::Stage4Params;
use crate::program_hold::Group;
use nord_core::cbin::ProgramDecoded;
use nord_core::engine::Stage4Engine;
use nord_core::piano::NpnoSound;
use nord_core::render_opts::FirmwareOptions;
use nord_core::synth::SynthSampleSound;
use nord_gui::audio::garbage::{Bin, Garbage};
use std::sync::Arc;

pub use crate::load_queue::{load_queue, LoadReceiver, LoadSender, Refused};

/// Jobs the queue holds before the editor reports the plugin busy.
///
/// ASSUMED: 16 is far above the burst of one library click (a program load plus up to two sound
/// banks). A full queue means `process()` is not running (host stopped or the plugin inactive).
pub const LOAD_QUEUE_CAPACITY: usize = 16;

/// A value prepared on the GUI side for the engine.
///
/// `FirmwareOptions` is `Copy` with no heap data, so the large variant is moved through the queue
/// by value: boxing it would make `process()` free the box.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum LoadJob {
    /// A decoded program (`AudioCommand::LoadProgram`).
    Program(Box<ProgramDecoded>),
    /// A pre-decoded `.npno` / SFZ piano for Piano A (`slot_b = false`) or B, or `None` to clear.
    Piano {
        slot_b: bool,
        sound: Option<Arc<NpnoSound>>,
    },
    /// A pre-decoded `.nsmp4` bank for Synth A, B or C (`layer` 0..=2), or `None` to clear.
    Sample {
        layer: usize,
        sound: Option<Arc<SynthSampleSound>>,
    },
    /// Every opt-in firmware path at once (`AudioCommand::SetFirmwareOptions`).
    Firmware(FirmwareOptions),
    /// The WAVE stage of the three synth layers on or off (`SynthLayer::wave_bank`), set by the
    /// library worker once the OS update tables are installed. Holds no parameter group.
    WaveBank(bool),
    /// `library_program` went to Off: drop the program and both sound banks sets and hand the
    /// engine back to the host parameters (D252). Allocates nothing; what it drops goes to the
    /// bin. Firmware options and the WAVE stage stay.
    Unload,
}

/// What has been loaded into the plugin, kept to restore a rebuilt engine.
#[derive(Default)]
pub struct Loaded {
    pub program: Option<Box<ProgramDecoded>>,
    pub pianos: [Option<Arc<NpnoSound>>; 2],
    pub samples: [Option<Arc<SynthSampleSound>>; 3],
    pub firmware: Option<FirmwareOptions>,
    /// `LoadJob::WaveBank`: the WAVE stage switch, re-applied after the firmware options.
    pub wave_bank: Option<bool>,
}

impl Loaded {
    /// Applies one job on the audio thread: moves it into the engine, records it, and hands
    /// everything it replaced to `bin`. No allocation, no free, no lock.
    pub fn apply(
        &mut self,
        job: LoadJob,
        engine: &mut Stage4Engine,
        params: &Stage4Params,
        bin: &Bin,
    ) {
        match job {
            LoadJob::Program(prog) => {
                engine.apply_program(&prog);
                // The program wins again: Touch / Mod 2 (D169) and every other group (D218) stay
                // the program's until the host moves them.
                params.program_overrides.rebaseline(params);
                params.program_hold.hold(Group::PROGRAM);
                // A restored project: the edits it had armed win over this program (D262).
                let (hold, overrides) = params.library_state.take_rearm();
                params.program_hold.arm(hold);
                params.program_overrides.arm(overrides);
                if let Some(old) = self.program.replace(prog) {
                    bin.put(Garbage::Program(old));
                }
            }
            LoadJob::Piano { slot_b, sound } => {
                let slot = usize::from(slot_b);
                let layer = if slot_b {
                    &mut engine.piano_b
                } else {
                    &mut engine.piano_a
                };
                // The engine and `self` share the bank: the previous one is freed off-thread.
                bin.put(Garbage::Piano(layer.replace_sound(sound.clone())));
                bin.put(Garbage::Piano(std::mem::replace(
                    &mut self.pianos[slot],
                    sound,
                )));
            }
            LoadJob::Sample { layer, sound } => {
                let Some(slot) = self.samples.get_mut(layer) else {
                    return;
                };
                let synth = match layer {
                    0 => &mut engine.synth_a,
                    1 => &mut engine.synth_b,
                    _ => &mut engine.synth_c,
                };
                bin.put(Garbage::Synth(synth.replace_sample(sound.clone())));
                bin.put(Garbage::Synth(std::mem::replace(slot, sound)));
            }
            LoadJob::Firmware(opts) => {
                opts.apply_exact(engine);
                params.program_hold.hold(Group::FIRMWARE);
                self.firmware = Some(opts);
                if let Some(on) = self.wave_bank {
                    set_wave_bank(engine, on);
                }
            }
            LoadJob::WaveBank(on) => {
                set_wave_bank(engine, on);
                self.wave_bank = Some(on);
            }
            LoadJob::Unload => {
                for slot_b in [false, true] {
                    self.apply(
                        LoadJob::Piano {
                            slot_b,
                            sound: None,
                        },
                        engine,
                        params,
                        bin,
                    );
                }
                for layer in 0..self.samples.len() {
                    self.apply(LoadJob::Sample { layer, sound: None }, engine, params, bin);
                }
                if let Some(old) = self.program.take() {
                    bin.put(Garbage::Program(old));
                }
                // The next block's sync writes every host parameter group again; the firmware
                // switches stay the firmware panel's.
                params.program_hold.release(Group::PROGRAM);
            }
        }
    }

    /// Puts everything loaded back into a freshly built engine. Not real-time safe (clones
    /// `Arc`s into the engine); runs where the engine is rebuilt (`initialize`, `reset`). The
    /// hold state lives in the parameters and is not touched.
    pub fn restore(&self, engine: &mut Stage4Engine) {
        if let Some(prog) = &self.program {
            engine.apply_program(prog);
        }
        engine.piano_a.set_sound(self.pianos[0].clone());
        engine.piano_b.set_sound(self.pianos[1].clone());
        engine.synth_a.load_sample(self.samples[0].clone());
        engine.synth_b.load_sample(self.samples[1].clone());
        engine.synth_c.load_sample(self.samples[2].clone());
        if let Some(opts) = &self.firmware {
            opts.apply_exact(engine);
        }
        if let Some(on) = self.wave_bank {
            set_wave_bank(engine, on);
        }
    }
}

/// The WAVE stage switch of the three synth layers (what `FirmwareOptions::synth_wave_bank`
/// sets; the tables themselves are process-global, `wave_tables.rs`).
fn set_wave_bank(engine: &mut Stage4Engine, on: bool) {
    engine.synth_a.wave_bank = on;
    engine.synth_b.wave_bank = on;
    engine.synth_c.wave_bank = on;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nord_gui::audio::garbage;

    fn params() -> Arc<Stage4Params> {
        let p = Arc::new(Stage4Params::default());
        p.program_hold.bind(&p);
        p
    }

    fn piano(name: &str) -> Arc<NpnoSound> {
        Arc::new(NpnoSound::new_synthetic(
            name,
            60,
            &[0.1, 0.2, 0.1, 0.0],
            48_000.0,
        ))
    }

    fn program(level: f32) -> Box<ProgramDecoded> {
        let mut prog = ProgramDecoded::default();
        prog.organ.layer_a_level = level;
        prog.organ.drawbars_a = [8, 8, 8, 0, 0, 0, 0, 0, 0];
        Box::new(prog)
    }

    #[test]
    fn a_program_job_loads_the_engine_holds_the_groups_and_keeps_the_box() {
        let p = params();
        let mut e = Stage4Engine::new(48_000.0);
        let mut loaded = Loaded::default();
        loaded.apply(LoadJob::Program(program(0.42)), &mut e, &p, &Bin::inline());
        assert_eq!(e.organ_a.drawbars, [8, 8, 8, 0, 0, 0, 0, 0, 0]);
        assert!((e.layer_levels[0].base - 0.42).abs() < 1e-5);
        assert!(loaded.program.is_some());
        assert!(p.program_hold.owned_by_program(Group::Organ));
        assert!(p.program_hold.runs(Group::Live));
        assert!(
            p.program_hold.runs(Group::Firmware),
            "a program leaves the firmware flags"
        );
    }

    #[test]
    fn the_block_sync_leaves_an_untouched_program_alone_and_the_host_wins_once_it_moves() {
        let p = params();
        let mut e = Stage4Engine::new(48_000.0);
        let mut loaded = Loaded::default();
        loaded.apply(LoadJob::Program(program(0.42)), &mut e, &p, &Bin::inline());
        p.sync_to_engine(&mut e);
        assert_eq!(
            e.organ_a.drawbars,
            [8, 8, 8, 0, 0, 0, 0, 0, 0],
            "program survived the sync"
        );
        assert!((e.layer_levels[0].base - 0.42).abs() < 1e-5);

        // The host moves a level: the Levels group is written from the host again, the rest holds.
        p.program_hold.set_test_value("synth_a_level", 0.5);
        p.sync_to_engine(&mut e);
        assert!(
            (e.layer_levels[0].base - 0.64).abs() < 1e-5,
            "levels: host values again"
        );
        assert_eq!(
            e.organ_a.drawbars,
            [8, 8, 8, 0, 0, 0, 0, 0, 0],
            "organ group still the program's"
        );
    }

    #[test]
    fn a_second_program_replaces_the_first_and_the_old_box_goes_to_the_bin() {
        let p = params();
        let mut e = Stage4Engine::new(48_000.0);
        let mut loaded = Loaded::default();
        let (bin, reaper) = garbage::channel();
        loaded.apply(LoadJob::Program(program(0.3)), &mut e, &p, &bin);
        assert_eq!(reaper.try_iter().count(), 0, "nothing was replaced yet");
        loaded.apply(LoadJob::Program(program(0.6)), &mut e, &p, &bin);
        let got: Vec<_> = reaper.try_iter().collect();
        assert_eq!(got.len(), 1);
        assert!(
            matches!(&got[0], Garbage::Program(old) if (old.organ.layer_a_level - 0.3).abs() < 1e-6)
        );
        assert!((loaded.program.as_ref().unwrap().organ.layer_a_level - 0.6).abs() < 1e-6);
    }

    #[test]
    fn an_unload_job_drops_the_program_and_banks_and_releases_the_hold() {
        let p = params();
        let mut e = Stage4Engine::new(48_000.0);
        let mut loaded = Loaded::default();
        let (bin, reaper) = garbage::channel();
        loaded.apply(LoadJob::Program(program(0.3)), &mut e, &p, &bin);
        loaded.apply(
            LoadJob::Piano {
                slot_b: false,
                sound: Some(piano("Grand")),
            },
            &mut e,
            &p,
            &bin,
        );
        assert!(p.program_hold.owned_by_program(Group::Synth));
        let _ = reaper.try_iter().count();
        loaded.apply(LoadJob::Unload, &mut e, &p, &bin);
        assert!(loaded.program.is_none());
        assert!(loaded.pianos.iter().all(Option::is_none));
        assert!(loaded.samples.iter().all(Option::is_none));
        assert!(p.program_hold.runs(Group::Synth));
        assert!(
            reaper.try_iter().count() >= 2,
            "program and bank are binned"
        );
    }

    #[test]
    fn piano_and_sample_jobs_swap_the_banks_and_bin_the_old_ones() {
        let p = params();
        let mut e = Stage4Engine::new(48_000.0);
        let mut loaded = Loaded::default();
        let (bin, reaper) = garbage::channel();
        loaded.apply(
            LoadJob::Piano {
                slot_b: true,
                sound: Some(piano("Grand")),
            },
            &mut e,
            &p,
            &bin,
        );
        assert!(loaded.pianos[1].is_some() && loaded.pianos[0].is_none());
        loaded.apply(
            LoadJob::Piano {
                slot_b: true,
                sound: None,
            },
            &mut e,
            &p,
            &bin,
        );
        assert!(loaded.pianos[1].is_none());
        let sample = Arc::new(SynthSampleSound {
            name: "S".into(),
            zones: vec![],
        });
        loaded.apply(
            LoadJob::Sample {
                layer: 2,
                sound: Some(sample),
            },
            &mut e,
            &p,
            &bin,
        );
        assert_eq!(e.synth_c.loaded_sample_name(), Some("S"));
        loaded.apply(
            LoadJob::Sample {
                layer: 9,
                sound: None,
            },
            &mut e,
            &p,
            &bin,
        );
        // The replaced banks were queued for the reaper, never dropped here (2 per job).
        assert_eq!(reaper.try_iter().count(), 2 + 2 + 2);
        assert!(
            p.program_hold.runs(Group::Organ),
            "sound banks hold no parameter group"
        );
    }

    #[test]
    fn a_firmware_job_applies_the_options_and_holds_only_the_firmware_flags() {
        let p = params();
        let mut e = Stage4Engine::new(48_000.0);
        let mut loaded = Loaded::default();
        let opts = FirmwareOptions {
            synth_wave_bank: true,
            ..FirmwareOptions::default()
        };
        loaded.apply(LoadJob::Firmware(opts), &mut e, &p, &Bin::inline());
        assert_eq!(loaded.firmware, Some(opts));
        assert!(p.program_hold.owned_by_program(Group::Firmware));
        assert!(p.program_hold.runs(Group::Organ));
    }

    #[test]
    fn a_wave_bank_job_switches_the_stage_holds_nothing_and_survives_firmware_and_rebuild() {
        let p = params();
        let mut e = Stage4Engine::new(48_000.0);
        let mut loaded = Loaded::default();
        assert!(!e.synth_a.wave_bank);
        loaded.apply(LoadJob::WaveBank(true), &mut e, &p, &Bin::inline());
        assert!(e.synth_a.wave_bank && e.synth_b.wave_bank && e.synth_c.wave_bank);
        assert!(p.program_hold.runs(Group::Firmware), "no group is held");
        // Firmware options with the stage off do not undo the switch.
        loaded.apply(
            LoadJob::Firmware(FirmwareOptions::default()),
            &mut e,
            &p,
            &Bin::inline(),
        );
        assert!(e.synth_c.wave_bank);
        let mut fresh = Stage4Engine::new(96_000.0);
        loaded.restore(&mut fresh);
        assert!(fresh.synth_b.wave_bank);
        loaded.apply(LoadJob::WaveBank(false), &mut e, &p, &Bin::inline());
        assert!(!e.synth_a.wave_bank);
    }

    #[test]
    fn restore_puts_everything_back_into_a_rebuilt_engine() {
        let p = params();
        let mut e = Stage4Engine::new(48_000.0);
        let mut loaded = Loaded::default();
        loaded.apply(LoadJob::Program(program(0.42)), &mut e, &p, &Bin::inline());
        loaded.apply(
            LoadJob::Piano {
                slot_b: false,
                sound: Some(piano("Grand")),
            },
            &mut e,
            &p,
            &Bin::inline(),
        );
        let mut fresh = Stage4Engine::new(96_000.0);
        loaded.restore(&mut fresh);
        assert_eq!(fresh.organ_a.drawbars, [8, 8, 8, 0, 0, 0, 0, 0, 0]);
        assert!(fresh.piano_a.sound.is_some());
    }
}
