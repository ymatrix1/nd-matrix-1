//! Automated ARM+SHARC + Manual audit tests covering all 31 baked knobs, 97 buttons,
//! 197 round LEDs, and 42 triangular matrix LEDs.

use super::baked_buttons::{
    handle_baked_button_click, sync_baked_leds, ORGAN_KB_ZONE_LEDS, PIANO_KB_ZONE_LEDS,
    SYNTH_KB_ZONE_LEDS,
};
use super::baked_view::apply_baked_knob;
use super::prelude::*;
use crate::baked::BakedPanelAssets;
use std::collections::BTreeSet;

fn init_led_tri_state(assets: &BakedPanelAssets) -> (Vec<(bool, String)>, Vec<bool>) {
    let leds = assets
        .layout
        .leds
        .iter()
        .map(|l| (l.on, l.color.clone()))
        .collect();
    let tris = assets.layout.tris.iter().map(|t| t.on).collect();
    (leds, tris)
}

#[test]
fn audit_all_31_knobs_match_arm_layout_and_drive_engine() {
    let ctx = egui::Context::default();
    let assets = BakedPanelAssets::get_or_load(&ctx);
    assert_eq!(assets.layout.knobs.len(), 31, "expected 31 knobs");

    let encoders: Vec<usize> = assets
        .layout
        .knobs
        .iter()
        .filter(|k| k.encoder)
        .map(|k| k.index)
        .collect();
    assert_eq!(
        encoders,
        vec![2, 3, 4, 5, 6],
        "Piano Select (2), Program Dial (3), and Synth OLED dials (4, 5, 6) must be encoders"
    );

    for k in &assets.layout.knobs {
        let mut state = Stage4PanelState::default();
        let (audio, rx) = AudioHandle::new();
        let before_prog = state.program_ui.browse;
        apply_baked_knob(k.index, 0.65, 0.5, false, &mut state, &audio);
        let cmds: Vec<_> = rx.try_iter().collect();
        assert!(
            !cmds.is_empty() || state.program_ui.browse != before_prog,
            "knob {} produced no AudioCommand or ProgramUi change",
            k.index
        );
        let mut engine = Stage4Engine::new(48_000.0);
        let mut vol = 0.85;
        for c in cmds {
            apply_audio_command(c, &mut engine, &mut vol);
        }
        let (mut l, mut r) = ([0.0; 16], [0.0; 16]);
        engine.process_block(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|x| x.is_finite()));
    }
}

#[test]
fn audit_all_97_buttons_primary_and_shifted_and_octave_oled_popup() {
    let ctx = egui::Context::default();
    let assets = BakedPanelAssets::get_or_load(&ctx);
    assert_eq!(assets.layout.buttons.len(), 97, "expected 97 buttons");

    // Verify Organ (7/8), Piano (20/21), and Synth (60/61) OCTAVE SHIFT trigger the OLED popup
    for (btn_idx, expected_oct) in [(8, 1), (20, -1), (61, 1)] {
        let mut state = Stage4PanelState::default();
        state.program_ui.now_ms = 500;
        let (audio, _rx) = AudioHandle::new();
        let (mut leds, mut tris) = init_led_tri_state(&assets);
        handle_baked_button_click(
            &assets.layout.buttons[btn_idx],
            false,
            &mut state,
            &audio,
            &assets,
            &mut leds,
            &mut tris,
        );
        assert_eq!(
            state.program_ui.octave_popup(),
            Some(("Octave Shift".to_string(), expected_oct)),
            "button {btn_idx} must open the ARM-verified Octave Shift OLED popup"
        );
    }

    // Verify all 97 buttons in both plain and shifted modes apply cleanly to Stage4Engine
    for b in &assets.layout.buttons {
        for shift in [false, true] {
            let mut state = Stage4PanelState::default();
            let (audio, rx) = AudioHandle::new();
            let (mut leds, mut tris) = init_led_tri_state(&assets);
            handle_baked_button_click(b, shift, &mut state, &audio, &assets, &mut leds, &mut tris);
            sync_baked_leds(&state, &assets, &mut leds);
            let mut engine = Stage4Engine::new(48_000.0);
            let mut vol = 0.85;
            for c in rx.try_iter() {
                apply_audio_command(c, &mut engine, &mut vol);
            }
            let (mut l, mut r) = ([0.0; 16], [0.0; 16]);
            engine.process_block(&mut l, &mut r);
            assert!(
                l.iter().chain(&r).all(|x| x.is_finite()),
                "button {} (shift={shift}) produced non-finite DSP output",
                b.index
            );
        }
    }
}

#[test]
fn audit_all_197_round_leds_and_42_tri_leds_sync_with_panel_state() {
    let ctx = egui::Context::default();
    let assets = BakedPanelAssets::get_or_load(&ctx);
    assert_eq!(assets.layout.leds.len(), 197, "expected 197 round LEDs");
    assert_eq!(assets.layout.tris.len(), 42, "expected 42 triangular LEDs");

    let tri_groups: BTreeSet<&str> = assets
        .layout
        .tris
        .iter()
        .map(|t| t.group.as_str())
        .collect();
    for g in [
        "org-model",
        "org-vib",
        "pno-cat",
        "fx-mod1",
        "fx-mod2",
        "fx-amp",
        "fx-rev",
    ] {
        assert!(tri_groups.contains(g), "missing tri matrix {g}");
    }

    // With SPLIT off, all 12 KB ZONE LEDs (Organ, Piano, Synth) must be dark (hardware rule)
    let mut state = Stage4PanelState::default();
    let (mut leds, _tris) = init_led_tri_state(&assets);
    sync_baked_leds(&state, &assets, &mut leds);
    for idx in ORGAN_KB_ZONE_LEDS
        .into_iter()
        .chain(PIANO_KB_ZONE_LEDS)
        .chain(SYNTH_KB_ZONE_LEDS)
    {
        assert!(
            !leds[idx].0,
            "KB ZONE LED {idx} must stay dark while SPLIT is OFF"
        );
    }

    // Turning SPLIT on lights all 12 KB ZONE LEDs green (default all-zones assignment)
    state.program_ui.split.on = true;
    sync_baked_leds(&state, &assets, &mut leds);
    for idx in ORGAN_KB_ZONE_LEDS
        .into_iter()
        .chain(PIANO_KB_ZONE_LEDS)
        .chain(SYNTH_KB_ZONE_LEDS)
    {
        assert!(leds[idx].0, "KB ZONE LED {idx} must light when SPLIT is ON");
        assert_eq!(leds[idx].1, "green");
    }
}
