//! Engine-level tests of the firmware arpeggiator walk (D166): Up and Down order over the pool,
//! the stored RANGE value reaching the walk (`arp_range_raw`, `firmware_range_raw`) and the
//! default engine playing the firmware walk after a program load.

use super::*;
use crate::cbin::synth_arp::SynthArpDecoded;
use crate::cbin::synth_preset::SynthPresetDecoded;
use crate::cbin::SynthLayerDecoded;
use crate::engine::Stage4Engine;

const FS: f32 = 48_000.0;

/// Notes the arpeggiator picks over `steps` steps (480 BPM: a step is 3,000 frames).
fn picked(synth: &mut SynthLayer, steps: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut last = synth.arp_step;
    while out.len() < steps {
        synth.render_frame();
        if synth.arp_step != last {
            last = synth.arp_step;
            out.extend(synth.arp_active_notes[0]);
        }
    }
    out
}

fn layer(mode: ArpMode, octaves: u8, extra_thirds: u8, raw: u8, keys: &[u8]) -> SynthLayer {
    let mut s = SynthLayer::new(FS);
    s.arp_mode = mode;
    s.arp_octave_range = octaves;
    s.perf.range_extra_thirds = extra_thirds;
    s.arp_range_raw = raw;
    s.arp_rate_bpm = 480.0;
    for &k in keys {
        s.note_on(k, 100);
    }
    s
}

#[test]
fn up_walks_the_pool_and_wraps_like_the_arm_cycle() {
    // ARM capture (arp-fw-range.json, C 60 64 67, Up, raw 47): 64 67 72 76 79 84 60 ... (the
    // capture starts on the second pool note).
    let up = picked(&mut layer(ArpMode::Up, 2, 0, 47, &[60, 64, 67]), 9);
    assert_eq!(up, [60, 64, 67, 72, 76, 79, 84, 60, 64]);
}

#[test]
fn down_walks_the_pool_from_the_top_root_like_the_arm_cycle() {
    // ARM capture (Down, raw 47): 79 76 72 67 64 60 84 79 76 72 (starts on the second note).
    let down = picked(&mut layer(ArpMode::Down, 2, 0, 47, &[60, 64, 67]), 8);
    assert_eq!(down, [84, 79, 76, 72, 67, 64, 60, 84]);
}

#[test]
fn a_single_held_key_walks_its_octaves_in_both_directions() {
    // ARM capture (C 60, raw 85 = 6 thirds): Up 72 84 96 60 ..., Down visits the same four notes.
    let up = picked(&mut layer(ArpMode::Up, 3, 0, 85, &[60]), 5);
    assert_eq!(up, [60, 72, 84, 96, 60]);
    let down = picked(&mut layer(ArpMode::Down, 3, 0, 85, &[60]), 5);
    assert_eq!(down, [96, 84, 72, 60, 96]);
}

#[test]
fn the_stored_range_raw_sets_the_span_while_it_matches_the_engine_range() {
    // raw 16 and 1 extra third: the stored value projects onto the engine range, so the fitted
    // span of raw 16 (16.5 semitones) applies: 72 and 76 join the triad, 79 does not.
    let mut s = layer(ArpMode::Up, 1, 1, 16, &[60, 64, 67]);
    assert_eq!(picked(&mut s, 6), [60, 64, 67, 72, 76, 60]);
    // The same engine range with another stored raw (an edit of the dial moved the thirds): the
    // nearest detent, 127 / 9 per third, is used instead.
    let mut s = layer(ArpMode::Up, 1, 0, 16, &[60, 64, 67]);
    assert_eq!(
        picked(&mut s, 4),
        [60, 64, 67, 60],
        "one octave after a range edit"
    );
}

#[test]
fn apply_arp_modes_stores_the_raw_range_for_the_walk() {
    let dec = SynthLayerDecoded {
        arp_enabled: true,
        arp: SynthArpDecoded {
            mode: 0,
            direction: 0,
            range: 47,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut s = SynthLayer::new(FS);
    crate::synth_params_arp::apply_arp_modes(&mut s, &dec);
    assert_eq!(s.arp_range_raw, 47);
    assert_eq!(s.arp_octave_range, 2);
    assert_eq!(s.perf.range_extra_thirds, 0);
    assert_eq!(s.arp_mode, ArpMode::Up);
}

#[test]
fn the_default_engine_plays_the_firmware_arp_after_a_program_load() {
    let mut preset = SynthPresetDecoded::default();
    preset.layer_c.arp_enabled = true;
    preset.layer_c.arp = SynthArpDecoded {
        mode: 0,
        direction: 0,
        range: 47,
        ..Default::default()
    };
    let mut e = Stage4Engine::new(FS);
    e.apply_synth_section(&preset);
    assert!(e.synth_c.arp_firmware_walk, "walk survives the layer init");
    assert!(!e.synth_c.arp_firmware_gate, "the gate stays opt-in");
    assert_eq!(e.synth_c.arp_range_raw, 47);
    e.synth_c.arp_rate_bpm = 480.0;
    for k in [60, 64, 67] {
        e.synth_c.note_on(k, 100);
    }
    let notes = picked(&mut e.synth_c, 8);
    assert_eq!(notes, [60, 64, 67, 72, 76, 79, 84, 60]);
    // `--legacy-arp` before the load keeps the engine walk through the layer init.
    let mut legacy = Stage4Engine::new(FS);
    crate::render_opts::FirmwareOptions::from_args(&["--legacy-arp".to_string()])
        .unwrap()
        .apply_to(&mut legacy);
    legacy.apply_synth_section(&preset);
    assert!(!legacy.synth_c.arp_firmware_walk);
}
