//! `library_program` = Off at plugin level: selecting Off releases the library program, and a
//! project saved with Off reopens without the default `A:11` (D252).

use crate::auto_library::support::two_program_backup;
use crate::auto_library::LibrarySource;
use crate::program_hold::Group;
use crate::NdMatrix1Plugin;
use nih_plug::prelude::*;
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::time::Duration;

pub(crate) struct Init;
impl InitContext<NdMatrix1Plugin> for Init {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }
    fn execute(&self, _task: ()) {}
    fn set_latency_samples(&self, _samples: u32) {}
    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

pub(crate) struct Ctx {
    pub(crate) transport: Transport,
}
impl ProcessContext<NdMatrix1Plugin> for Ctx {
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

/// What a host project load does: replaces the stored value (`set_plain_value` is crate-private
/// in nih-plug, so the param object is replaced by one whose current value is the new one).
pub(crate) fn set_program(plugin: &mut NdMatrix1Plugin, value: i32) {
    let p = Arc::get_mut(&mut plugin.params).expect("no editor holds the params in this test");
    p.library_program = IntParam::new(
        "Program",
        value,
        IntRange::Linear {
            min: 0,
            max: crate::library_select::TOTAL,
        },
    );
}

pub(crate) fn initialize(plugin: &mut NdMatrix1Plugin) {
    let cfg = BufferConfig {
        sample_rate: 48000.0,
        min_buffer_size: None,
        max_buffer_size: 512,
        process_mode: ProcessMode::Offline,
    };
    assert!(plugin.initialize(&NdMatrix1Plugin::AUDIO_IO_LAYOUTS[0], &cfg, &mut Init));
}

pub(crate) fn block(plugin: &mut NdMatrix1Plugin) {
    let mut l = [0.0f32; 64];
    let mut r = [0.0f32; 64];
    let slices: Vec<&mut [f32]> = vec![&mut l[..], &mut r[..]];
    let mut buffer = Buffer::default();
    unsafe {
        buffer.set_slices(64, |output| *output = slices);
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    // SAFETY: an all-zero `Transport` is valid (plain numbers and `None`s), as in the lib tests.
    let mut transport: Transport = unsafe { MaybeUninit::zeroed().assume_init() };
    transport.sample_rate = 48000.0;
    let mut ctx = Ctx { transport };
    plugin.process(&mut buffer, &mut aux, &mut ctx);
}

pub(crate) fn program_name(plugin: &NdMatrix1Plugin) -> Option<String> {
    plugin.loaded.program.as_ref().map(|p| p.name.clone())
}

#[test]
fn selecting_off_releases_the_library_program_and_its_hold() {
    let path = two_program_backup("off-release");
    let mut plugin = NdMatrix1Plugin {
        library_source: LibrarySource::Path(path.clone()),
        ..Default::default()
    };
    initialize(&mut plugin);
    assert_eq!(program_name(&plugin).as_deref(), Some("First"), "A:11");
    assert!(plugin.params.program_hold.owned_by_program(Group::Synth));

    set_program(&mut plugin, 0);
    block(&mut plugin);
    let link = plugin.library.as_ref().expect("worker");
    assert!(link.wait_handled(0, Duration::from_secs(10)));
    block(&mut plugin);
    let _ = std::fs::remove_file(&path);
    assert_eq!(program_name(&plugin), None, "Off drops the program");
    assert!(
        !plugin.params.program_hold.owned_by_program(Group::Synth),
        "host parameters drive the engine again"
    );
}

#[test]
fn a_project_saved_with_off_reopens_without_the_default_program() {
    let path = two_program_backup("off-saved");
    let mut plugin = NdMatrix1Plugin {
        library_source: LibrarySource::Path(path.clone()),
        ..Default::default()
    };
    set_program(&mut plugin, 0); // the host restored the saved state before `initialize`
    initialize(&mut plugin);
    block(&mut plugin);
    let _ = std::fs::remove_file(&path);
    assert_eq!(program_name(&plugin), None);
    assert!(!plugin.params.program_hold.owned_by_program(Group::Synth));
}

#[test]
fn a_state_saved_without_the_parameter_keeps_the_default_a11() {
    // A project saved before `library_program` existed has no stored value: the parameter stays
    // at its default and the first program of the library loads (D252, guide).
    let path = two_program_backup("off-migrate");
    let mut plugin = NdMatrix1Plugin {
        library_source: LibrarySource::Path(path.clone()),
        ..Default::default()
    };
    assert_eq!(plugin.params.library_program.value(), 1);
    initialize(&mut plugin);
    let _ = std::fs::remove_file(&path);
    assert_eq!(program_name(&plugin).as_deref(), Some("First"));
}
