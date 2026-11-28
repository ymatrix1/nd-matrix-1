//! Loaded program versus host parameters (D169): the program's piano Touch and firmware Mod 2 play
//! in the plugin until the host moves the matching parameter; old sessions render as before.
//!
//! Own test binary: drives `process()` the way the wrappers do and restores old state through
//! replaced param values, like a host project load (`Param::set_plain_value` is crate-private).

use nih_plug::prelude::*;
use nord_core::cbin::ProgramDecoded;
use nord_core::piano::VelocityTouch;
use nord_plugin::fx_selectors::Mod2Sel;
use nord_plugin::NdMatrix1Plugin;
use std::collections::VecDeque;
use std::mem::MaybeUninit;
use std::sync::Arc;

struct Init;
impl InitContext<NdMatrix1Plugin> for Init {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }
    fn execute(&self, _task: ()) {}
    fn set_latency_samples(&self, _samples: u32) {}
    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

struct Ctx {
    transport: Transport,
    events: VecDeque<NoteEvent<()>>,
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
        self.events.pop_front()
    }
    fn send_event(&mut self, _event: NoteEvent<()>) {}
    fn set_latency_samples(&self, _samples: u32) {}
    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

fn new_ctx() -> Ctx {
    // SAFETY: `Transport` is plain data; all-zero is a valid value.
    let mut transport: Transport = unsafe { MaybeUninit::zeroed().assume_init() };
    transport.sample_rate = 48_000.0;
    transport.tempo = Some(120.0);
    Ctx {
        transport,
        events: VecDeque::new(),
    }
}

fn config() -> BufferConfig {
    BufferConfig {
        sample_rate: 48_000.0,
        min_buffer_size: None,
        max_buffer_size: 256,
        process_mode: ProcessMode::Realtime,
    }
}

fn activate() -> NdMatrix1Plugin {
    let mut plugin = NdMatrix1Plugin::default();
    let layout = &NdMatrix1Plugin::AUDIO_IO_LAYOUTS[0];
    plugin.library_source = nord_plugin::auto_library::LibrarySource::None;
    assert!(plugin.initialize(layout, &config(), &mut Init));
    plugin
}

/// One 256-frame `process()` block; returns the summed absolute output.
fn process(plugin: &mut NdMatrix1Plugin, ctx: &mut Ctx) -> f32 {
    let (mut l, mut r) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    let mut buffer = Buffer::default();
    // SAFETY: the slices outlive `buffer` and are exactly 256 frames long.
    unsafe {
        buffer.set_slices(256, |o| *o = vec![&mut l[..], &mut r[..]]);
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    plugin.process(&mut buffer, &mut aux, ctx);
    l.iter().chain(r.iter()).map(|s| s.abs()).sum()
}

/// Moves a host parameter to `text` (the value a host automation or project load would set).
/// `Param::set_plain_value` is crate-private in nih-plug, so the param object is replaced by one
/// whose current value is the new one; `value()` is all the plugin reads. Only the ids used here.
fn set_param(plugin: &mut NdMatrix1Plugin, id: &str, text: &str) {
    assert!(
        plugin.params.param_map().iter().any(|(i, _, _)| i == id),
        "{id} is not a host parameter id"
    );
    let p = Arc::get_mut(&mut plugin.params).expect("no editor holds the params in this test");
    let int = |v: &str, max| {
        IntParam::new(
            id.to_string(),
            v.parse().unwrap(),
            IntRange::Linear { min: 0, max },
        )
    };
    match id {
        "piano_touch" => p.piano_touch = int(text, 2),
        "piano_category" => p.piano_category = int(text, 5),
        "master_level" => {
            p.master_level = FloatParam::new(
                "Master",
                text.parse().unwrap(),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
        }
        "fxsel_mod2_rate_3" => {
            p.layer_fx_selectors.layers[2].mod2_rate_hz = FloatParam::new(
                "Rate",
                text.parse().unwrap(),
                FloatRange::Linear {
                    min: 0.1,
                    max: 10.0,
                },
            )
        }
        "fxsel_mod2_type_3" => {
            let v = Mod2Sel::variants().iter().position(|n| *n == text).unwrap();
            p.layer_fx_selectors.layers[2].mod2_type =
                EnumParam::new("Type", Mod2Sel::from_index(v));
        }
        other => panic!("fixture id {other} has no setter"),
    }
}

/// A program with per-layer Touch (A Light, B Heavy) and a firmware Mod 2 on Piano A.
fn program() -> ProgramDecoded {
    let mut p = ProgramDecoded::default();
    p.piano.layer_a.touch = VelocityTouch::Light;
    p.piano.layer_b.touch = VelocityTouch::Heavy;
    p.piano.fx_a.mod2_enabled = true;
    p.piano.fx_a.mod2_amount = 0.7;
    p.piano.fx_a.mod2_rate = 0.5;
    p.piano.fx_a.mod2_type_raw = 6;
    p
}

fn touches(plugin: &NdMatrix1Plugin) -> (VelocityTouch, VelocityTouch) {
    (plugin.engine.piano_a.touch, plugin.engine.piano_b.touch)
}

fn play_piano(plugin: &mut NdMatrix1Plugin, velocity: f32) -> f32 {
    let mut ctx = new_ctx();
    ctx.events.push_back(NoteEvent::NoteOn {
        timing: 0,
        voice_id: None,
        channel: 0,
        note: 60,
        velocity,
    });
    (0..8).map(|_| process(plugin, &mut ctx)).sum()
}

#[test]
fn program_touch_and_mod2_words_survive_the_block_sync() {
    let mut plugin = activate();
    plugin.load_program(program());
    let mut ctx = new_ctx();
    process(&mut plugin, &mut ctx);
    process(&mut plugin, &mut ctx);
    assert_eq!(
        touches(&plugin),
        (VelocityTouch::Light, VelocityTouch::Heavy)
    );
    let fx = &plugin.engine.fx_chains[2];
    assert_eq!(fx.mod2_type_raw, Some(6), "firmware Mod 2 word kept");
    assert!(fx.mod2_rate_raw.is_some() && fx.firmware_fx.mod2);
}

#[test]
fn program_touch_changes_the_rendered_signal() {
    let render = |touch| {
        let mut plugin = activate();
        let mut prog = program();
        prog.piano.layer_a.touch = touch;
        prog.piano.layer_b.touch = touch;
        plugin.load_program(prog);
        play_piano(&mut plugin, 0.4)
    };
    let (heavy, light) = (render(VelocityTouch::Heavy), render(VelocityTouch::Light));
    assert!(heavy > 0.0 && light > 0.0, "heavy {heavy}, light {light}");
    assert!(
        (heavy - light).abs() > 1e-3 * light,
        "program Touch must reach the render: heavy {heavy}, light {light}"
    );
}

#[test]
fn a_host_move_wins_over_the_program_and_keeps_winning() {
    let mut plugin = activate();
    let mut prog = program();
    prog.piano.fx_b.mod2_type_raw = 4;
    plugin.load_program(prog);
    let mut ctx = new_ctx();
    process(&mut plugin, &mut ctx);
    set_param(&mut plugin, "piano_touch", "2");
    set_param(&mut plugin, "fxsel_mod2_type_3", "Chorus");
    process(&mut plugin, &mut ctx);
    assert_eq!(
        touches(&plugin),
        (VelocityTouch::Heavy, VelocityTouch::Heavy)
    );
    let fx = &plugin.engine.fx_chains[2];
    assert_eq!(fx.mod2_type_raw, None, "host Mod 2 replaces program words");
    assert_eq!(fx.mod2_type, nord_core::fx::Mod2Type::Chorus);
    // Back to the default value: still the host's choice (an undo is a host change).
    set_param(&mut plugin, "piano_touch", "0");
    process(&mut plugin, &mut ctx);
    assert_eq!(
        touches(&plugin),
        (VelocityTouch::Medium, VelocityTouch::Medium)
    );
    // Another layer's Mod 2 was never touched by the host.
    assert_eq!(plugin.engine.fx_chains[3].mod2_type_raw, Some(4));
}

#[test]
fn loading_a_program_ends_the_host_override() {
    let mut plugin = activate();
    set_param(&mut plugin, "piano_touch", "2");
    let mut ctx = new_ctx();
    process(&mut plugin, &mut ctx);
    assert_eq!(
        touches(&plugin),
        (VelocityTouch::Heavy, VelocityTouch::Heavy)
    );
    plugin.load_program(program());
    process(&mut plugin, &mut ctx);
    assert_eq!(
        touches(&plugin),
        (VelocityTouch::Light, VelocityTouch::Heavy)
    );
}

#[test]
fn a_host_reset_keeps_the_loaded_program() {
    let mut plugin = activate();
    plugin.load_program(program());
    plugin.reset();
    let mut ctx = new_ctx();
    process(&mut plugin, &mut ctx);
    assert_eq!(
        touches(&plugin),
        (VelocityTouch::Light, VelocityTouch::Heavy)
    );
    assert_eq!(plugin.engine.fx_chains[2].mod2_type_raw, Some(6));
}

#[test]
fn an_untouched_host_without_a_program_equals_the_engine_defaults() {
    let mut plugin = activate();
    let mut ctx = new_ctx();
    process(&mut plugin, &mut ctx);
    let reference = nord_core::engine::Stage4Engine::new(48_000.0);
    assert_eq!(
        touches(&plugin),
        (reference.piano_a.touch, reference.piano_b.touch)
    );
    for (a, b) in plugin.engine.fx_chains.iter().zip(&reference.fx_chains) {
        assert_eq!((a.mod2_type, a.mod2_rate_hz), (b.mod2_type, b.mod2_rate_hz));
        assert_eq!(a.mod2_type_raw, b.mod2_type_raw);
    }
    assert!(plugin.engine.firmware_morph && plugin.engine.synth_a.arp_firmware_walk);
}

/// Old saved session (fixture, display strings by id): restored values count as host changes, so
/// they play exactly as before D169.
#[test]
fn an_old_session_restores_its_touch_and_mod2_as_before() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/old_state_v1.json")).unwrap();
    let mut plugin = NdMatrix1Plugin::default();
    for (id, text) in fixture.as_object().unwrap() {
        if !id.starts_with('_') {
            set_param(&mut plugin, id, text.as_str().unwrap());
        }
    }
    let layout = &NdMatrix1Plugin::AUDIO_IO_LAYOUTS[0];
    plugin.library_source = nord_plugin::auto_library::LibrarySource::None;
    assert!(plugin.initialize(layout, &config(), &mut Init));
    let mut ctx = new_ctx();
    process(&mut plugin, &mut ctx);
    assert_eq!(
        touches(&plugin),
        (VelocityTouch::Heavy, VelocityTouch::Heavy)
    );
    let fx = &plugin.engine.fx_chains[2];
    assert_eq!(fx.mod2_type, nord_core::fx::Mod2Type::Chorus);
    assert!((fx.mod2_rate_hz - 3.0).abs() < 0.05, "{}", fx.mod2_rate_hz);
    assert_eq!(fx.mod2_type_raw, None);
}
