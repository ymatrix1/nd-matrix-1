//! Nord Stage 4 DAW Plugin implementation.
//!
//! Provides CLAP and VST3 wrappers around `Stage4Engine` with host automation,
//! multitimbral MIDI handling, sample rate safety, zero-allocation audio callback,
//! and state persistence.

#![allow(unknown_lints)]
#![allow(semicolon_in_expressions_from_macros)]
#![allow(semicolon_in_expressions_from_non_local_macros)]

pub mod audio_out;
pub mod auto_library;
pub mod aux_pedal_params;
mod core_sync;
pub mod editor;
pub mod firmware_params;
pub mod fx_extra_params;
pub mod fx_params;
pub mod fx_selectors;
pub mod fx_sharing_params;
pub mod fx_variation_params;
pub mod library_host;
#[cfg(test)]
mod library_off_tests;
pub mod library_select;
pub mod lle_bridge;
pub mod load_queue;
pub mod loader;
pub mod menu_extra_params;
pub mod midi;
pub mod midi_menu_params;
pub mod morph_assign;
pub mod organ_extra_params;
pub mod organ_live_params;
pub mod organ_params;
pub mod output_params;
pub mod param_snapshot;
pub mod params;
pub mod pedal_assign_params;
pub mod piano_extra_params;
pub mod piano_params;
mod plugin_ids;
pub mod program_hold;
pub mod program_overrides;
pub mod program_params;
pub mod settings_params;
pub mod shared_sounds;
pub mod state;
pub mod synth_gate_params;
pub mod synth_mod_params;
pub mod synth_params;
pub mod synth_pattern_params;
pub mod wave_tables;

use nih_plug::prelude::*;
use nord_core::cbin::{CbinError, CbinFile, ProgramDecoded};
use nord_core::engine::{Stage4Engine, TelemetryCell};
use nord_gui::audio::garbage::{spawn_reaper, Bin};
use params::Stage4Params;
use std::sync::Arc;

/// The Nord Stage 4 DAW Plugin.
pub struct NdMatrix1Plugin {
    pub params: Arc<Stage4Params>,
    pub engine: Stage4Engine,
    pub sample_rate: f32,
    /// What was loaded through [`loader`] (program, sound banks, firmware options), re-applied
    /// whenever the engine is rebuilt (`initialize`, `reset`) so a host reset does not drop it.
    pub loaded: loader::Loaded,
    /// Where the audio thread puts what it must not free (replaced programs and sound banks).
    pub bin: Bin,
    /// Producer end of the keybed queue, handed to the editor.
    pub note_tx: editor::notes::NoteSender,
    /// Consumer end, drained at the start of every `process()` block.
    pub note_rx: editor::notes::NoteReceiver,
    /// Producer end of the load queue (programs, sounds, firmware), handed to the editor.
    pub load_tx: loader::LoadSender,
    /// Consumer end, drained at the start of every `process()` block.
    pub load_rx: loader::LoadReceiver,
    /// Where the engine publishes its state (lit keys, LEDs, ladders) for the editor to read.
    /// It outlives engine rebuilds: each new engine is attached to the same cell.
    pub telemetry: Arc<TelemetryCell>,
    /// The library worker serving `library_program` (`auto_library.rs`); `None` without a saved
    /// library. Started when the instance activates.
    pub library: Option<auto_library::LibraryLink>,
    /// The `library_program` value last sent to the worker; a change in `process()` sends again.
    last_library_program: i32,
    /// Where the library comes from: the user's saved one, or what a test or tool sets.
    pub library_source: auto_library::LibrarySource,
    /// Opt-in live I/O bridge to the LLE layer (`lle_bridge`); off unless requested.
    pub lle: lle_bridge::LleBridge,
}

impl Default for NdMatrix1Plugin {
    fn default() -> Self {
        let (note_tx, note_rx) = editor::notes::note_queue(editor::notes::NOTE_QUEUE_CAPACITY);
        let (load_tx, load_rx) = loader::load_queue(loader::LOAD_QUEUE_CAPACITY);
        // The params sit at their final address inside the `Arc`: build the hold table now.
        let params = Arc::new(Stage4Params::default());
        params.program_hold.bind(&params);
        let telemetry = Arc::new(TelemetryCell::default());
        let mut engine = Stage4Engine::new(44100.0);
        engine.attach_telemetry(telemetry.clone());
        Self {
            params,
            engine,
            sample_rate: 44100.0,
            loaded: loader::Loaded::default(),
            bin: Bin::inline(),
            note_tx,
            note_rx,
            load_tx,
            load_rx,
            telemetry,
            library: None,
            last_library_program: -1,
            library_source: auto_library::LibrarySource::Saved,
            lle: lle_bridge::LleBridge::off(),
        }
    }
}

impl NdMatrix1Plugin {
    /// Fresh engine at the current sample rate with everything loaded applied, then the host
    /// parameters synced on top (D169 / D218: only what the host moved replaces the program).
    fn rebuild_engine(&mut self) {
        self.engine = Stage4Engine::new(self.sample_rate);
        self.engine.attach_telemetry(self.telemetry.clone());
        self.loaded.restore(&mut self.engine);
        self.params.sync_to_engine(&mut self.engine);
    }

    /// Applies a decoded program to the engine and makes it win over every host parameter group
    /// until the host moves one (D169, D218). Frees the replaced program here: use it from the
    /// thread that owns the plugin while it is not processing (tests); the editor goes through
    /// the load queue instead.
    pub fn load_program(&mut self, prog: ProgramDecoded) {
        self.loaded.apply(
            loader::LoadJob::Program(Box::new(prog)),
            &mut self.engine,
            &self.params,
            &Bin::inline(),
        );
    }

    /// Parses `.ns4p` bytes and calls [`Self::load_program`].
    pub fn load_program_bytes(&mut self, bytes: &[u8]) -> Result<(), CbinError> {
        let prog = ProgramDecoded::from_cbin_file(&CbinFile::parse(bytes)?)?;
        self.load_program(prog);
        Ok(())
    }

    /// Applies every queued editor load (`process()` start): no allocation, no free, no lock.
    pub(crate) fn drain_loads(&mut self) {
        while let Some(job) = self.load_rx.pop() {
            self.loaded
                .apply(job, &mut self.engine, &self.params, &self.bin);
        }
    }
}

impl Plugin for NdMatrix1Plugin {
    const NAME: &'static str = "ND Matrix 1";
    const VENDOR: &'static str = "ND Matrix Contributors";
    const URL: &'static str = "https://github.com/ymatrix1/nd-matrix-1";
    const EMAIL: &'static str = "";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Stereo instrument: main 1-2 plus aux 3-4 first, stereo only as the fallback.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = audio_out::LAYOUTS;

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.note_tx.clone(),
            self.load_tx.clone(),
            self.telemetry.clone(),
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        // An active instance gets a reaper thread (it frees what `process()` replaces); a plugin
        // that is only scanned never starts one.
        self.bin = spawn_reaper();
        // The saved Nord OS update's WAVE tables, parsed off-thread and installed once per process.
        wave_tables::install_saved_in_background();
        // The saved sound library: the wanted program is decoded off-thread, queued, and
        // applied before the first block (the host may render at once).
        if self.start_library(self.library_source.path()) {
            auto_library::report(&format!(
                "initialize: {:?} at {} Hz, loading program {}",
                buffer_config.process_mode,
                buffer_config.sample_rate,
                library_select::label(self.last_library_program)
            ));
            self.wait_for_library_program();
        }
        self.rebuild_engine();
        // Opt-in LLE live I/O (`NS4_ENGINE=lle` or the saved engine setting); off otherwise.
        self.lle = lle_bridge::LleBridge::from_process_settings();
        self.lle.bind_params(&self.params);
        if let Some(text) = self.lle.status_text() {
            nih_log!("{text}");
        }
        true
    }

    fn reset(&mut self) {
        // Reset engine states upon host reset / playhead reposition
        self.rebuild_engine();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // A moved Program parameter asks the worker; editor and worker loads are applied first,
        // so the block sync below sees the program that was just loaded.
        self.request_library_program();
        self.drain_loads();

        // Synchronize automatable parameters to engine state
        self.params.sync_to_engine(&mut self.engine);

        // Synchronize host transport tempo if available
        if let Some(transport) = context.transport().tempo {
            self.engine.master_clock.bpm = transport as f32;
        }

        let mode = self.params.midi_mode.value();

        // On-screen keybed events queued by the editor (lock-free, no allocation).
        while let Some(ev) = self.note_rx.pop() {
            editor::notes::apply_to_engine(&mut self.engine, &self.params, mode, ev);
        }

        // Process incoming MIDI and note events
        let mut next_event = context.next_event();
        let num_samples = buffer.samples();
        let mut sample_idx = 0;
        // Params the host or editor moved since the last block (a no-op unless the bridge is on).
        self.lle.scan_params();

        let channels = buffer.as_slice();
        let mut aux_channels = aux.outputs.first_mut().map(|b| b.as_slice());

        while sample_idx < num_samples {
            // Dispatch all events at the current sample offset
            while let Some(event) = next_event {
                if event.timing() as usize <= sample_idx {
                    self.lle.forward_host_event(&event);
                    midi::route_plugin_event(&mut self.engine, &self.params, mode, event);
                    next_event = context.next_event();
                } else {
                    break;
                }
            }

            // Determine block end (up to next event timing or end of buffer)
            let block_end = if let Some(ref event) = next_event {
                (event.timing() as usize).min(num_samples)
            } else {
                num_samples
            };

            // Render into the host buffers without heap allocations. With the aux port present
            // (2-bus layout) outputs 3-4 go to it; otherwise only outputs 1-2 are rendered.
            let range = sample_idx..block_end;
            if channels.len() >= 2 {
                let (left, right) = channels.split_at_mut(1);
                let (ml, mr) = (&mut left[0][range.clone()], &mut right[0][range.clone()]);
                let aux_pair = aux_channels.as_deref_mut().and_then(|a| match a {
                    [al, ar, ..] => Some((al.get_mut(range.clone())?, ar.get_mut(range)?)),
                    _ => None,
                });
                match aux_pair {
                    Some((al, ar)) => audio_out::render_main_aux(&mut self.engine, ml, mr, al, ar),
                    None => audio_out::render_main(&mut self.engine, ml, mr),
                }
            } else if channels.len() == 1 {
                audio_out::render_mono(&mut self.engine, &mut channels[0][range]);
            }

            sample_idx = block_end;
        }

        // One snapshot per block for the editor (lock-free seqlock write, no allocation).
        self.engine.publish_telemetry();
        self.lle.end_block();

        ProcessStatus::Normal
    }

    fn deactivate(&mut self) {}
}

impl ClapPlugin for NdMatrix1Plugin {
    const CLAP_ID: &'static str = "org.ndmatrix.nd-matrix-1";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Open-source 7-layer multitimbral keyboard emulator");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for NdMatrix1Plugin {
    const VST3_CLASS_ID: [u8; 16] = *b"NDMatrix1Emulato";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(NdMatrix1Plugin);
nih_export_vst3!(NdMatrix1Plugin);

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::MaybeUninit;

    struct MockInitContext;
    impl InitContext<NdMatrix1Plugin> for MockInitContext {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _task: ()) {}
        fn set_latency_samples(&self, _samples: u32) {}
        fn set_current_voice_capacity(&self, _capacity: u32) {}
    }

    struct MockProcessContext {
        transport: Transport,
    }

    impl ProcessContext<NdMatrix1Plugin> for MockProcessContext {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute_background(&self, _task: ()) {}
        fn execute_gui(&self, _task: ()) {}
        fn transport(&self) -> &Transport {
            &self.transport
        }
        fn next_event(&mut self) -> Option<NoteEvent<()>> {
            None
        }
        fn send_event(&mut self, _event: NoteEvent<()>) {}
        fn set_latency_samples(&self, _samples: u32) {}
        fn set_current_voice_capacity(&self, _capacity: u32) {}
    }

    #[test]
    fn test_editor_keybed_note_reaches_process_output() {
        let mut plugin = NdMatrix1Plugin::default();
        let mut init_ctx = MockInitContext;
        plugin.library_source = crate::auto_library::LibrarySource::None;
        plugin.initialize(
            &NdMatrix1Plugin::AUDIO_IO_LAYOUTS[0],
            &BufferConfig {
                sample_rate: 48000.0,
                min_buffer_size: None,
                max_buffer_size: 512,
                process_mode: ProcessMode::Realtime,
            },
            &mut init_ctx,
        );
        assert!(plugin.note_tx.push(nord_core::midi::MidiEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        }));

        let mut peak = 0.0f32;
        let mut transport: Transport = unsafe { MaybeUninit::zeroed().assume_init() };
        transport.sample_rate = 48000.0;
        let mut context = MockProcessContext { transport };
        for _ in 0..32 {
            let mut left_buf = [0.0f32; 128];
            let mut right_buf = [0.0f32; 128];
            let channel_slices: Vec<&mut [f32]> = vec![&mut left_buf[..], &mut right_buf[..]];
            let mut buffer = Buffer::default();
            unsafe {
                buffer.set_slices(128, |output| {
                    *output = channel_slices;
                });
            }
            let mut aux = AuxiliaryBuffers {
                inputs: &mut [],
                outputs: &mut [],
            };
            plugin.process(&mut buffer, &mut aux, &mut context);
            peak = buffer
                .as_slice()
                .iter()
                .flat_map(|c| c.iter())
                .fold(peak, |m, v| m.max(v.abs()));
        }
        assert!(plugin.note_rx.is_empty(), "process() must drain the queue");
        assert!(peak > 0.0, "queued key press produced no audio");
        // The editor's lit keys and LEDs read this cell: process() must have published into it.
        let snap =
            nord_gui::telemetry::TelemetryHandle::from_cell(plugin.telemetry.clone()).latest();
        assert!(
            snap.get().is_some_and(|t| t.note_held(60)),
            "process() did not publish the held key to the editor"
        );
    }

    /// Host-visible metadata must point at the live repository and carry no manufacturer name.
    #[test]
    fn test_plugin_metadata_url_matches_cargo_repository_and_has_no_vendor_name() {
        assert_eq!(NdMatrix1Plugin::URL, env!("CARGO_PKG_REPOSITORY"));
        let shown = [
            NdMatrix1Plugin::NAME,
            NdMatrix1Plugin::VENDOR,
            NdMatrix1Plugin::URL,
            <NdMatrix1Plugin as ClapPlugin>::CLAP_ID,
        ];
        for text in shown {
            assert!(!text.to_lowercase().contains("clavia"), "{text}");
        }
    }

    /// Contact metadata carries no address on a domain the project does not own, and the
    /// host-facing identifiers stay frozen so saved projects keep loading (D16).
    #[test]
    fn test_plugin_contact_metadata_and_stable_host_ids() {
        assert_eq!(
            NdMatrix1Plugin::URL,
            "https://github.com/ymatrix1/nd-matrix-1"
        );
        assert_eq!(NdMatrix1Plugin::EMAIL, "");
        assert_eq!(env!("CARGO_PKG_AUTHORS"), "ymatrix1");
        assert!(!env!("CARGO_PKG_AUTHORS").contains('@'));
        // VERIFIED: unchanged from the pre-rename release (D16), checked by this test.
        assert_eq!(NdMatrix1Plugin::NAME, "ND Matrix 1");
        assert_eq!(
            <NdMatrix1Plugin as ClapPlugin>::CLAP_ID,
            "org.ndmatrix.nd-matrix-1"
        );
        assert_eq!(
            &<NdMatrix1Plugin as Vst3Plugin>::VST3_CLASS_ID,
            b"NDMatrix1Emulato"
        );
    }

    #[test]
    fn test_plugin_instantiation_and_reset() {
        let mut plugin = NdMatrix1Plugin::default();
        assert_eq!(plugin.sample_rate, 44100.0);
        plugin.sample_rate = 96000.0;
        plugin.reset();
        assert_eq!(plugin.engine.sample_rate, 96000.0);
    }

    #[test]
    fn test_plugin_process_buffer_zero_alloc() {
        let mut plugin = NdMatrix1Plugin::default();
        let mut init_ctx = MockInitContext;
        plugin.library_source = crate::auto_library::LibrarySource::None;
        plugin.initialize(
            &NdMatrix1Plugin::AUDIO_IO_LAYOUTS[0],
            &BufferConfig {
                sample_rate: 48000.0,
                min_buffer_size: None,
                max_buffer_size: 512,
                process_mode: ProcessMode::Realtime,
            },
            &mut init_ctx,
        );

        let mut left_buf = [0.0f32; 128];
        let mut right_buf = [0.0f32; 128];
        let channel_slices: Vec<&mut [f32]> = vec![&mut left_buf[..], &mut right_buf[..]];
        let mut buffer = Buffer::default();
        unsafe {
            buffer.set_slices(128, |output| {
                *output = channel_slices;
            });
        }

        let mut aux = AuxiliaryBuffers {
            inputs: &mut [],
            outputs: &mut [],
        };

        let mut transport: Transport = unsafe { MaybeUninit::zeroed().assume_init() };
        transport.sample_rate = 48000.0;
        transport.tempo = Some(132.0);

        let mut context = MockProcessContext { transport };
        let status = plugin.process(&mut buffer, &mut aux, &mut context);
        assert_eq!(status, ProcessStatus::Normal);
        assert_eq!(plugin.engine.master_clock.bpm, 132.0);
    }
}
