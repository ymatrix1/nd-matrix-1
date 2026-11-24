//! Complete 1:1 button dispatch and real-time LED synchronization for the baked panel view.
//!
//! Wires all 97 buttons (including Organ/Piano/Synth `OCTAVE SHIFT` / `KB ZONE`, `SPLIT`,
//! `BANK/PAGE`, `STORE`, `LIVE MODE`, `SCENE II`, Piano Timbre/Acoustics/Unison/Touch/Comp,
//! Synth OSC/Arp/Voice/Vib/LFO/Filter/Amp/Unison, and all FX focus/on/model switches) to
//! `Stage4PanelState` and `AudioHandle`, and syncs the 197 panel LEDs (including the 3×4 green
//! `KB ZONE` LEDs and 10 keybed split LEDs) every frame.

use super::kb_zone;
use super::organ_logic::step_octave;
use super::piano_logic;
use super::prelude::*;
use super::ui_state::Btn as SynthBtn;
use crate::audio::{FxEdit, FxFlag, OrganEdit, PianoEdit};
use crate::baked::{BakedButton, BakedPanelAssets};
use nord_core::fx::{AmpCabinet, DelayFeedbackFx, DelayFilterMode, ReverbColor};
use nord_panel::behaviour::Trigger;

/// Organ, Piano, and Synth `KB ZONE` LED index quartets in `panel_layout.json`.
pub(crate) const ORGAN_KB_ZONE_LEDS: [usize; 4] = [16, 17, 18, 19];
pub(crate) const PIANO_KB_ZONE_LEDS: [usize; 4] = [37, 38, 39, 40];
pub(crate) const SYNTH_KB_ZONE_LEDS: [usize; 4] = [92, 93, 94, 95];
/// Split count LEDs (1/2/3 split points) in `bay-program`.
const SPLIT_COUNT_LEDS: [usize; 3] = [53, 54, 55];
/// 10 red split-point LEDs above the keybed (`y=413`).
const KEYBED_SPLIT_LEDS: [usize; 10] = [187, 188, 189, 190, 191, 192, 193, 194, 195, 196];
/// Program 1..=8 LED indices in `bay-program`.
const PROG_SLOT_LEDS: [usize; 8] = [68, 69, 70, 71, 72, 73, 74, 75];

fn fx_slot_of(state: &Stage4PanelState) -> u8 {
    match state.fx_focus {
        0 => 0,
        1 => 1 + state.piano_ui.layer().min(1) as u8,
        _ => 3 + state.synth_ui.focus.min(2) as u8,
    }
}

fn send_synth_btn(state: &mut Stage4PanelState, audio: &AudioHandle, btn: SynthBtn, trig: Trigger) {
    state.synth_ui.press(btn, trig);
    for edit in state.synth_ui.take_edits() {
        audio.send_command(AudioCommand::Synth(edit));
    }
}

fn send_program_action(state: &mut Stage4PanelState, audio: &AudioHandle, id: &str, trig: Trigger) {
    for cmd in state.program_ui.activate(id, trig) {
        audio.send_command(cmd);
    }
    let loc = state.program_ui.loc;
    state.prog_id = loc.label();
    let name = super::bay_program::oled::name_at(&state.program_ui, loc);
    if !name.is_empty() {
        state.prog_title = name;
    }
}

/// Optional hover tooltip for buttons whose Shift or secondary function isn't obvious from the cap.
pub(crate) fn button_tooltip(idx: usize, state: &Stage4PanelState) -> Option<String> {
    match idx {
        7 | 8 => Some(format!(
            "Organ OCTAVE SHIFT {:+} (click: octave, Shift+click: KB ZONE)",
            state.organ_octave_of(state.organ_ui.focus_layer)
        )),
        20 | 21 => Some(format!(
            "Piano OCTAVE SHIFT {:+} (click: octave, Shift+click: KB ZONE)",
            state.piano_ui.octave[state.piano_ui.layer()]
        )),
        60 | 61 => Some(format!(
            "Synth OCTAVE SHIFT {:+} (click: octave, Shift+click: KB ZONE)",
            state.synth_ui.layer().octave
        )),
        30 => Some(format!(
            "SPLIT {} (click to toggle keyboard split & KB ZONE LEDs)",
            if state.program_ui.split.on {
                "ON"
            } else {
                "OFF"
            }
        )),
        53 => Some(format!(
            "SHIFT {} (latch Shift for secondary functions like KB ZONE)",
            if state.piano_ui.shift_latch {
                "LATCHED"
            } else {
                "OFF"
            }
        )),
        _ => None,
    }
}

/// Synchronize dynamic LEDs in `led_states` with `Stage4PanelState` so every LED matches the live
/// engine state (including after `.ns4p` preset loads, `SPLIT`, `OCTAVE SHIFT`, and `KB ZONE`).
pub(crate) fn sync_baked_leds(
    state: &Stage4PanelState,
    assets: &BakedPanelAssets,
    led_states: &mut [(bool, String)],
) {
    let set_by_id = |leds: &mut [(bool, String)], id: &str, on: bool| {
        if let Some(i) = assets
            .layout
            .leds
            .iter()
            .position(|l| l.id.as_deref() == Some(id))
        {
            leds[i].0 = on;
        }
    };

    // Section & layer activity LEDs
    set_by_id(led_states, "org-on", state.organ_section_on());
    set_by_id(led_states, "org-a", state.organ_a_active);
    set_by_id(led_states, "org-b", state.organ_b_active);
    set_by_id(
        led_states,
        "pno-on",
        state.piano_a_active || state.piano_b_active,
    );
    set_by_id(led_states, "pno-a", state.piano_a_active);
    set_by_id(led_states, "pno-b", state.piano_b_active);
    set_by_id(
        led_states,
        "syn-on",
        state.synth_a_active || state.synth_b_active || state.synth_c_active,
    );
    set_by_id(led_states, "syn-a", state.synth_a_active);
    set_by_id(led_states, "syn-b", state.synth_b_active);
    set_by_id(led_states, "syn-c", state.synth_c_active);

    // Program slots 1..=8 & mode LEDs
    let slot = usize::from(state.program_ui.loc.slot.min(7));
    for (i, &li) in PROG_SLOT_LEDS.iter().enumerate() {
        if li < led_states.len() {
            led_states[li].0 = i == slot;
        }
    }
    set_by_id(
        led_states,
        "live-led",
        state.program_ui.live.is_active() || state.program_ui.live_armed,
    );
    set_by_id(led_states, "scene2-led", state.program_ui.scene2);
    set_by_id(led_states, "transp-top-led", state.program_ui.transp.on);
    set_by_id(led_states, "m-whl", state.morph_wheel_active);
    set_by_id(led_states, "m-at", state.morph_at_active);
    set_by_id(led_states, "m-ped", state.morph_ctrl_active);

    // Split count LEDs (3 red dots above SPLIT)
    let split_leds = state.program_ui.split.leds();
    for (i, &li) in SPLIT_COUNT_LEDS.iter().enumerate() {
        if li < led_states.len() {
            led_states[li].0 = split_leds[i];
        }
    }

    // 10 red keybed split-point LEDs (C2..F6 across the 88-key bed)
    const SPLIT_ANCHORS: [u8; 10] = [36, 41, 48, 53, 60, 65, 72, 77, 84, 89];
    for (i, &li) in KEYBED_SPLIT_LEDS.iter().enumerate() {
        if li < led_states.len() {
            let mut lit = false;
            if state.program_ui.split.on {
                for sp in 0..3 {
                    if state.program_ui.split.enabled[sp] {
                        let note = state.program_ui.split.notes[sp];
                        let nearest = SPLIT_ANCHORS
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, &a)| a.abs_diff(note))
                            .map(|(idx, _)| idx);
                        if nearest == Some(i) {
                            lit = true;
                        }
                    }
                }
            }
            led_states[li].0 = lit;
        }
    }

    // Organ, Piano, Synth 4-LED green KB ZONE indicators
    let split_on = state.program_ui.split.on;
    let org_zone = kb_zone::shown(
        kb_zone::lits_of(state.organ_ui.kb_zone[state.organ_ui.focus_layer]),
        split_on,
    );
    for (i, &li) in ORGAN_KB_ZONE_LEDS.iter().enumerate() {
        if li < led_states.len() {
            led_states[li] = (org_zone[i], "green".to_string());
        }
    }
    let pno_zone = kb_zone::shown(
        kb_zone::lits(state.piano_ui.kb_zone[state.piano_ui.layer()]),
        split_on,
    );
    for (i, &li) in PIANO_KB_ZONE_LEDS.iter().enumerate() {
        if li < led_states.len() {
            led_states[li] = (pno_zone[i], "green".to_string());
        }
    }
    let syn_zone = kb_zone::shown(kb_zone::lits(state.synth_ui.layer().kb_zone), split_on);
    for (i, &li) in SYNTH_KB_ZONE_LEDS.iter().enumerate() {
        if li < led_states.len() {
            led_states[li] = (syn_zone[i], "green".to_string());
        }
    }
}

/// Dispatch a click on baked button `b` to `Stage4PanelState`, `AudioHandle`, and `led_states` /
/// `tri_states`.
pub(crate) fn handle_baked_button_click(
    b: &BakedButton,
    shift_key: bool,
    state: &mut Stage4PanelState,
    audio_handle: &AudioHandle,
    assets: &BakedPanelAssets,
    led_states: &mut [(bool, String)],
    tri_states: &mut [bool],
) {
    let shift = shift_key || state.piano_ui.shift_latch;
    let trig = if shift {
        Trigger::Shifted
    } else {
        Trigger::Primary
    };
    let find_led = |id: &str| {
        assets
            .layout
            .leds
            .iter()
            .position(|l| l.id.as_deref() == Some(id))
    };

    // 1. Index-based handlers for buttons without HTML data-* attributes (OCTAVE SHIFT, BANK, EXIT, etc.)
    match b.index {
        // Organ OCTAVE SHIFT - / + (Shift: KB ZONE - / +)
        7 | 8 => {
            let up = b.index == 8;
            let focus = state.organ_ui.focus_layer;
            if shift {
                let pos = kb_zone::position_of(state.organ_ui.kb_zone[focus]);
                let z = kb_zone::step(pos, up);
                for t in state.edit_layers(focus, 2) {
                    state.organ_ui.kb_zone[t] = kb_zone::leds(z);
                    audio_handle.send_command(AudioCommand::Organ(OrganEdit::Zone {
                        layer: t as u8,
                        leds: kb_zone::leds(z),
                    }));
                }
                state.piano_ui.shift_latch = false;
            } else {
                let v = step_octave(state.organ_octave_of(focus), up);
                for t in state.edit_layers(focus, 2) {
                    state.set_organ_octave(t, v);
                    audio_handle.send_command(AudioCommand::Organ(OrganEdit::Octave {
                        layer: t as u8,
                        shift: v,
                    }));
                }
                state
                    .program_ui
                    .popups
                    .show_octave(v, state.program_ui.now_ms);
            }
            return;
        }
        // Piano OCTAVE SHIFT - / + (Shift: KB ZONE - / +)
        20 | 21 => {
            let up = b.index == 21;
            let mut edits = Vec::new();
            if shift {
                piano_logic::zone_step(state, up, &mut edits);
                state.piano_ui.shift_latch = false;
            } else {
                piano_logic::octave_step(state, if up { 1 } else { -1 }, &mut edits);
                let v = state.piano_ui.octave[state.piano_ui.layer()];
                state
                    .program_ui
                    .popups
                    .show_octave(v, state.program_ui.now_ms);
            }
            for e in edits {
                audio_handle.send_command(AudioCommand::Piano(e));
            }
            return;
        }
        // Synth OCTAVE SHIFT - / + (Shift: KB ZONE - / +)
        60 | 61 => {
            let btn = if b.index == 60 {
                SynthBtn::OctaveDown
            } else {
                SynthBtn::OctaveUp
            };
            send_synth_btn(state, audio_handle, btn, trig);
            if shift {
                state.piano_ui.shift_latch = false;
            } else {
                let v = state.synth_ui.layer().octave;
                state
                    .program_ui
                    .popups
                    .show_octave(v, state.program_ui.now_ms);
            }
            return;
        }
        // Program: MST CLK tap
        31 => {
            send_program_action(state, audio_handle, "mst_clk.tap_set", trig);
            return;
        }
        // Program: BANK / PAGE - and +
        34 | 35 => {
            let id = match (b.index == 35, shift) {
                (false, false) => "program.page_down",
                (true, false) => "program.page_up",
                (false, true) => "program.bank_down",
                (true, true) => "program.bank_up",
            };
            send_program_action(state, audio_handle, id, Trigger::Primary);
            state.piano_ui.shift_latch = false;
            return;
        }
        // Program: EXIT
        49 => {
            state.program_ui.exit();
            state.piano_ui.shift_latch = false;
            return;
        }
        // Program: PROG VIEW / LIST
        50 => {
            send_program_action(state, audio_handle, "program.prog_view", trig);
            return;
        }
        // Program: MON/COPY (Shift: PASTE)
        52 => {
            let id = if shift {
                "program.paste"
            } else {
                "program.mon_copy"
            };
            send_program_action(state, audio_handle, id, Trigger::Primary);
            state.piano_ui.shift_latch = false;
            return;
        }
        // Program: SHIFT latch
        53 => {
            state.piano_ui.shift_latch = !state.piano_ui.shift_latch;
            return;
        }
        // Synth: ARP MENU / PATTERN
        65 => {
            send_synth_btn(state, audio_handle, SynthBtn::ArpMenu, trig);
            return;
        }
        // FX: VARIATION / GLOBAL
        82 => {
            let slot = fx_slot_of(state);
            audio_handle.send_command(AudioCommand::Fx(FxEdit::Flag {
                slot,
                param: FxFlag::Mod1Variation,
                on: !shift,
            }));
            return;
        }
        // FX: DELAY TAP / ANALOG
        90 => {
            let slot = fx_slot_of(state);
            let on = if let Some(li) = find_led("dly-ana") {
                led_states[li].0 = !led_states[li].0;
                led_states[li].0
            } else {
                true
            };
            audio_handle.send_command(AudioCommand::Fx(FxEdit::Flag {
                slot,
                param: FxFlag::DelayAnalog,
                on,
            }));
            return;
        }
        _ => {}
    }

    // 2. Toggle LED buttons
    if let Some(id) = b.toggle_led.as_deref() {
        if let Some(li) = find_led(id) {
            led_states[li].0 = !led_states[li].0;
            let on = led_states[li].0;
            let slot = fx_slot_of(state);
            match id {
                "org-on" => {
                    state.organ_a_active = on;
                    state.organ_b_active = on;
                    audio_handle.send_command(AudioCommand::SetLayerActive(0, on));
                    audio_handle.send_command(AudioCommand::SetLayerActive(1, on));
                }
                "org-a" => {
                    state.organ_a_active = on;
                    state.organ_ui.focus_layer = 0;
                    audio_handle.send_command(AudioCommand::SetLayerActive(0, on));
                }
                "org-b" => {
                    state.organ_b_active = on;
                    state.organ_ui.focus_layer = 1;
                    audio_handle.send_command(AudioCommand::SetLayerActive(1, on));
                }
                "pno-on" => {
                    state.piano_a_active = on;
                    state.piano_b_active = on;
                    audio_handle.send_command(AudioCommand::SetLayerActive(2, on));
                    audio_handle.send_command(AudioCommand::SetLayerActive(3, on));
                }
                "pno-a" => {
                    state.piano_a_active = on;
                    state.piano_ui.focus = 0;
                    audio_handle.send_command(AudioCommand::SetLayerActive(2, on));
                }
                "pno-b" => {
                    state.piano_b_active = on;
                    state.piano_ui.focus = 1;
                    audio_handle.send_command(AudioCommand::SetLayerActive(3, on));
                }
                "syn-on" => {
                    state.synth_a_active = on;
                    state.synth_b_active = on;
                    state.synth_c_active = on;
                    audio_handle.send_command(AudioCommand::SetLayerActive(4, on));
                    audio_handle.send_command(AudioCommand::SetLayerActive(5, on));
                    audio_handle.send_command(AudioCommand::SetLayerActive(6, on));
                }
                "syn-a" => {
                    state.synth_a_active = on;
                    state.synth_ui.focus_layer(0);
                    audio_handle.send_command(AudioCommand::SetLayerActive(4, on));
                }
                "syn-b" => {
                    state.synth_b_active = on;
                    state.synth_ui.focus_layer(1);
                    audio_handle.send_command(AudioCommand::SetLayerActive(5, on));
                }
                "syn-c" => {
                    state.synth_c_active = on;
                    state.synth_ui.focus_layer(2);
                    audio_handle.send_command(AudioCommand::SetLayerActive(6, on));
                }
                "m-whl" => state.morph_wheel_active = on,
                "m-at" => state.morph_at_active = on,
                "m-ped" => state.morph_ctrl_active = on,
                "rotary-on" | "rotary-organ" => {
                    audio_handle.send_command(AudioCommand::SetRotaryEnabled(on));
                }
                "rotary-stop" => {
                    state.rotary_stop = on;
                    audio_handle.send_command(AudioCommand::SetRotaryEnabled(!on));
                }
                "org-vib-on" => {
                    state.scanner_on = on;
                    audio_handle.send_command(AudioCommand::SetScannerOn(on));
                }
                "perc-on" | "perc-soft" | "perc-fast" | "perc-third" => {
                    match id {
                        "perc-on" => state.perc_on = on,
                        "perc-soft" => state.perc_soft = on,
                        "perc-fast" => state.perc_fast = on,
                        "perc-third" => state.perc_third = on,
                        _ => {}
                    }
                    audio_handle.send_command(AudioCommand::SetPercussion {
                        enabled: state.perc_on,
                        soft: state.perc_soft,
                        fast: state.perc_fast,
                        third: state.perc_third,
                    });
                }
                "transp-top-led" => {
                    send_program_action(state, audio_handle, "transp.on_set", trig);
                }
                "store-led" => {
                    let pid = if shift {
                        "program.store_as"
                    } else {
                        "program.store"
                    };
                    send_program_action(state, audio_handle, pid, Trigger::Primary);
                }
                "live-led" => {
                    send_program_action(state, audio_handle, "program.live_mode", trig);
                }
                "scene2-led" => {
                    send_program_action(state, audio_handle, "program.layer_scene_ii", trig);
                }
                "secedit-led" => {
                    send_program_action(state, audio_handle, "program.section_edit", trig);
                }
                "plib-org" => {
                    send_program_action(state, audio_handle, "program.preset_organ", trig);
                }
                "plib-pno" => {
                    send_program_action(state, audio_handle, "program.preset_piano", trig);
                }
                "plib-syn" => {
                    send_program_action(state, audio_handle, "program.preset_synth", trig);
                }
                "kb-hold" => send_synth_btn(state, audio_handle, SynthBtn::KbHold, trig),
                "arp-run" => send_synth_btn(state, audio_handle, SynthBtn::ArpRun, trig),
                "arp-mode" => send_synth_btn(state, audio_handle, SynthBtn::ArpMode, trig),
                "syn-keep" => send_synth_btn(state, audio_handle, SynthBtn::Waveform, trig),
                "vib-menu" => send_synth_btn(state, audio_handle, SynthBtn::VibratoMenu, trig),
                "lfo-wf" => send_synth_btn(state, audio_handle, SynthBtn::LfoWaveform, trig),
                "osc-p" => send_synth_btn(state, audio_handle, SynthBtn::PitchSmp, trig),
                "osc-e" => send_synth_btn(state, audio_handle, SynthBtn::OscEnvelope, trig),
                "flt-t" => send_synth_btn(state, audio_handle, SynthBtn::FilterType, trig),
                "flt-e" => send_synth_btn(state, audio_handle, SynthBtn::FilterEnvelope, trig),
                "flt-on" => {
                    audio_handle.send_command(AudioCommand::SetSynthCutoff(if on {
                        state.synth_cutoff
                    } else {
                        1.0
                    }));
                }
                "mod1-on" => {
                    audio_handle.send_command(AudioCommand::Fx(FxEdit::Mod1Type {
                        slot,
                        kind: if on { state.mod1_type } else { Mod1Type::Off },
                    }));
                }
                "mod2-on" => {
                    audio_handle.send_command(AudioCommand::Fx(FxEdit::Mod2Type {
                        slot,
                        kind: if on { state.mod2_type } else { Mod2Type::Off },
                    }));
                }
                "amp-on" => {
                    audio_handle.send_command(AudioCommand::Fx(FxEdit::Cabinet {
                        slot,
                        cabinet: if on {
                            AmpCabinet::Twin
                        } else {
                            AmpCabinet::Off
                        },
                    }));
                }
                "dly-on" => {
                    state.delay_on = on;
                    audio_handle.send_command(AudioCommand::Fx(FxEdit::Flag {
                        slot,
                        param: FxFlag::DelayEnabled,
                        on,
                    }));
                    audio_handle.send_command(AudioCommand::SetFxDelay {
                        enabled: on,
                        feedback: state.delay_feedback,
                        mix: state.delay_mix,
                    });
                }
                "comp-on" => {
                    audio_handle.send_command(AudioCommand::Fx(FxEdit::Flag {
                        slot,
                        param: FxFlag::CompEnabled,
                        on,
                    }));
                }
                "rev-on" => {
                    audio_handle.send_command(AudioCommand::Fx(FxEdit::ReverbModel {
                        slot,
                        model: if on {
                            state.reverb_model
                        } else {
                            ReverbModel::Off
                        },
                    }));
                }
                _ => {}
            }
        }
    }

    // 3. Toggle pair buttons (`rotary-slow,rotary-fast`, `syn-mode-ana,syn-mode-smp`)
    if let Some(pair) = b.toggle_pair.as_deref() {
        let mut parts = pair.split(',');
        if let (Some(a_id), Some(b_id)) = (parts.next(), parts.next()) {
            if let (Some(ai), Some(bi)) = (find_led(a_id), find_led(b_id)) {
                let a_on = led_states[ai].0;
                led_states[ai].0 = !a_on;
                led_states[bi].0 = a_on;
                if a_id == "rotary-slow" {
                    state.rotary_speed = if !a_on {
                        RotarySpeed::Slow
                    } else {
                        RotarySpeed::Fast
                    };
                    audio_handle.send_command(AudioCommand::SetRotarySpeed(state.rotary_speed));
                } else if a_id == "syn-mode-ana" || a_id == "syn-mode-smp" {
                    send_synth_btn(state, audio_handle, SynthBtn::Mode, trig);
                    state.synth_osc_mode = state.synth_ui.layer().osc_mode();
                    audio_handle.send_command(AudioCommand::SetSynthOscMode(state.synth_osc_mode));
                }
            }
        }
    }

    // 4. Cycle LEDs (`pno-t-brt,...`, `pno-softrel,...`, `v-mono,v-leg`, `vib-*`, `dly-fx*`, `rev-brt,rev-drk`)
    if let Some(cycle) = b.cycle_leds.as_deref() {
        let indices: Vec<usize> = cycle.split(',').filter_map(find_led).collect();
        if !indices.is_empty() {
            let cur = indices.iter().position(|&li| led_states[li].0);
            let mut next = cur.map_or(0, |c| c + 1);
            if next >= indices.len() {
                next = if b.allow_off { usize::MAX } else { 0 };
            }
            for (pos, &li) in indices.iter().enumerate() {
                led_states[li].0 = pos == next;
            }
            let slot = fx_slot_of(state);
            match cycle {
                "pno-t-brt,pno-t-mid,pno-t-sft" => {
                    state.piano_timbre = match next {
                        0 => PianoTimbre::Bright,
                        1 => PianoTimbre::Mid,
                        2 => PianoTimbre::Soft,
                        _ => PianoTimbre::Off,
                    };
                    audio_handle.send_command(AudioCommand::SetPianoTimbre(state.piano_timbre));
                }
                "pno-softrel,pno-strres,pno-pednoise" => {
                    let layer = state.piano_ui.layer() as u8;
                    let soft = next == 0;
                    state.string_res = next == 1;
                    state.pedal_noise = next == 2;
                    audio_handle.send_command(AudioCommand::Piano(PianoEdit::SoftRelease {
                        layer,
                        on: soft,
                    }));
                    audio_handle.send_command(AudioCommand::SetPianoAcoustics {
                        string_res: state.string_res,
                        pedal_noise: state.pedal_noise,
                    });
                }
                "v-mono,v-leg" => send_synth_btn(state, audio_handle, SynthBtn::VoiceMode, trig),
                "vib-whl,vib-dly,vib-on" => {
                    send_synth_btn(state, audio_handle, SynthBtn::VibratoSel, trig);
                }
                "dly-fx1,dly-fx2,dly-fx3" => {
                    let fx = match next {
                        0 => DelayFeedbackFx::Ensemble,
                        1 => DelayFeedbackFx::Chorus,
                        2 => DelayFeedbackFx::Space,
                        _ => DelayFeedbackFx::Off,
                    };
                    audio_handle.send_command(AudioCommand::Fx(FxEdit::DelayFx { slot, fx }));
                }
                "rev-brt,rev-drk" => {
                    let color = match next {
                        0 => ReverbColor::Bright,
                        1 => ReverbColor::Dark,
                        _ => ReverbColor::Normal,
                    };
                    audio_handle
                        .send_command(AudioCommand::Fx(FxEdit::ReverbColor { slot, color }));
                }
                _ => {}
            }
        }
    }

    // 5. 2-LED 4-state cycles (`pno-uni`, `pno-touch`, `pno-comp`, `lfo-d`, `amp-v`, `syn-uni`, `dly-hp,dly-lp`)
    if let Some(c3) = b.cycle_3.as_deref() {
        let mut parts = c3.split(',');
        if let (Some(id1), Some(id2)) = (parts.next(), parts.next()) {
            if let (Some(i1), Some(i2)) = (find_led(id1), find_led(id2)) {
                let cur = match (led_states[i1].0, led_states[i2].0) {
                    (false, false) => 0,
                    (true, false) => 1,
                    (false, true) => 2,
                    (true, true) => 3,
                };
                let next = (cur + 1) % 4;
                led_states[i1].0 = next == 1 || next == 3;
                led_states[i2].0 = next == 2 || next == 3;
                let p_layer = state.piano_ui.layer() as u8;
                let slot = fx_slot_of(state);
                match c3 {
                    "pno-uni-1,pno-uni-2" => {
                        audio_handle.send_command(AudioCommand::Piano(PianoEdit::Unison {
                            layer: p_layer,
                            amount: next as u8,
                        }));
                    }
                    "pno-touch-m,pno-touch-h" => {
                        state.piano_touch = match next {
                            1 => VelocityTouch::Medium,
                            2 | 3 => VelocityTouch::Heavy,
                            _ => VelocityTouch::Light,
                        };
                        audio_handle.send_command(AudioCommand::SetPianoTouch(state.piano_touch));
                    }
                    "pno-comp-1,pno-comp-2" => {
                        audio_handle.send_command(AudioCommand::Piano(PianoEdit::DynComp {
                            layer: p_layer,
                            amount: next as u8,
                        }));
                    }
                    "lfo-d1,lfo-d2" => {
                        send_synth_btn(state, audio_handle, SynthBtn::LfoDest, trig);
                    }
                    "amp-v1,amp-v2" => {
                        send_synth_btn(state, audio_handle, SynthBtn::AmpEnvelope, trig);
                    }
                    "syn-uni-1,syn-uni-2" => {
                        send_synth_btn(state, audio_handle, SynthBtn::Unison, trig);
                    }
                    "dly-hp,dly-lp" => {
                        let mode = match next {
                            1 => DelayFilterMode::Hp,
                            2 => DelayFilterMode::Lp,
                            3 => DelayFilterMode::Bp,
                            _ => DelayFilterMode::Off,
                        };
                        audio_handle
                            .send_command(AudioCommand::Fx(FxEdit::DelayFilter { slot, mode }));
                    }
                    _ => {}
                }
            }
        }
    }

    // 6. Program 1..=8 buttons
    if let Some(p_num) = b.prog_btn {
        let mut target_loc = state.program_ui.loc;
        target_loc.slot = p_num.saturating_sub(1).min(7) as u8;
        for cmd in state.program_ui.select_loc(target_loc) {
            audio_handle.send_command(cmd);
        }
        state.prog_id = target_loc.label();
        if !state.program_ui.title.is_empty() {
            state.prog_title = state.program_ui.title.clone();
        }
    }

    // 7. SPLIT button (activates real split routing + lights split LEDs & KB ZONE LEDs)
    if b.split_btn {
        send_program_action(state, audio_handle, "split.on_set", trig);
    }

    // 8. FX FOCUS buttons (Organ / Piano / Synth)
    if let Some(target) = b.fx_focus.as_deref() {
        state.fx_focus = match target {
            "organ" => 0,
            "piano" => 1,
            _ => 2,
        };
        let groups: [(&str, &str, &[&str], &str); 3] = [
            ("organ", "org-fx-focus", &["fxf-org-a", "fxf-org-b"], "accent"),
            (
                "piano",
                "pno-fx-focus",
                &["fxf-pno-a", "fxf-pno-b"],
                "green",
            ),
            (
                "synth",
                "syn-fx-focus",
                &["fxf-syn-a", "fxf-syn-b", "fxf-syn-c"],
                "amber",
            ),
        ];
        for (name, hdr, strip, col) in groups {
            let active = name == target;
            if let Some(hi) = find_led(hdr) {
                led_states[hi] = (active, col.to_string());
            }
            for &sid in strip {
                if let Some(si) = find_led(sid) {
                    led_states[si] = (active, col.to_string());
                }
            }
        }
    }

    // 9. Triangular LED matrix cycle buttons
    if let Some(group_name) = b.cycle_btn.as_deref() {
        let mut cur: Option<usize> = None;
        let mut count: usize = 6;
        for (ti, t) in assets.layout.tris.iter().enumerate() {
            if t.group == group_name {
                count = t.count.max(1);
                if tri_states[ti] {
                    cur = Some(t.tri_idx);
                }
            }
        }
        let next = cur.map_or(0, |c| (c + 1) % count);
        for (ti, t) in assets.layout.tris.iter().enumerate() {
            if t.group == group_name {
                tri_states[ti] = t.tri_idx == next;
            }
        }
        let slot = fx_slot_of(state);
        match group_name {
            "org-model" => {
                let models = [
                    OrganModel::Farfisa,
                    OrganModel::Vox,
                    OrganModel::B3,
                    OrganModel::Pipe1,
                    OrganModel::Pipe2,
                    OrganModel::B3,
                ];
                state.organ_model = models[next % models.len()];
                audio_handle.send_command(AudioCommand::SetOrganModel(state.organ_model));
            }
            "org-vib" => {
                let modes = [
                    ScannerMode::C2,
                    ScannerMode::V3,
                    ScannerMode::C3,
                    ScannerMode::V2,
                    ScannerMode::C1,
                    ScannerMode::V1,
                ];
                state.scanner_mode = modes[next % modes.len()];
                audio_handle.send_command(AudioCommand::SetScannerMode(state.scanner_mode));
            }
            "pno-cat" => {
                let cats = [
                    PianoCategory::Electric,
                    PianoCategory::Upright,
                    PianoCategory::Grand,
                    PianoCategory::Clav,
                    PianoCategory::Digital,
                    PianoCategory::Misc,
                ];
                state.piano_cat = cats[next % cats.len()];
                audio_handle.send_command(AudioCommand::SetPianoCategory(state.piano_cat));
            }
            "fx-mod1" => {
                let kinds = [
                    Mod1Type::AutoPan,
                    Mod1Type::Tremolo,
                    Mod1Type::RingMod,
                    Mod1Type::AutoWah,
                    Mod1Type::Wah,
                    Mod1Type::Pump,
                ];
                state.mod1_type = kinds[next % kinds.len()];
                audio_handle.send_command(AudioCommand::SetFxMod1 {
                    kind: state.mod1_type,
                    rate: state.mod1_rate,
                    amount: state.mod1_amount,
                });
            }
            "fx-mod2" => {
                let kinds = [
                    Mod2Type::Phaser,
                    Mod2Type::Flanger,
                    Mod2Type::Chorus,
                    Mod2Type::Vibe,
                    Mod2Type::Ensemble,
                    Mod2Type::Spin,
                ];
                state.mod2_type = kinds[next % kinds.len()];
                audio_handle.send_command(AudioCommand::SetFxMod2 {
                    kind: state.mod2_type,
                    rate: state.mod2_rate,
                    amount: state.mod2_amount,
                });
            }
            "fx-amp" => {
                let cabs = [
                    AmpCabinet::Small,
                    AmpCabinet::Jc,
                    AmpCabinet::Twin,
                    AmpCabinet::ToRotary,
                    AmpCabinet::LpFilter,
                    AmpCabinet::HpFilter,
                ];
                audio_handle.send_command(AudioCommand::Fx(FxEdit::Cabinet {
                    slot,
                    cabinet: cabs[next % cabs.len()],
                }));
            }
            "fx-rev" => {
                let models = [
                    ReverbModel::Room,
                    ReverbModel::Booth,
                    ReverbModel::Spring,
                    ReverbModel::Stage,
                    ReverbModel::Hall,
                    ReverbModel::Cathedral,
                ];
                state.reverb_model = models[next % models.len()];
                audio_handle.send_command(AudioCommand::SetFxReverb {
                    model: state.reverb_model,
                    mix: state.reverb_mix,
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octave_shift_and_kb_zone_buttons_and_leds_work_for_all_three_sections() {
        let ctx = egui::Context::default();
        let assets = BakedPanelAssets::get_or_load(&ctx);
        let mut state = Stage4PanelState::default();
        let (audio, rx) = AudioHandle::new();
        let mut leds: Vec<(bool, String)> = assets
            .layout
            .leds
            .iter()
            .map(|l| (l.on, l.color.clone()))
            .collect();
        let mut tris: Vec<bool> = assets.layout.tris.iter().map(|t| t.on).collect();

        // Turn SPLIT on first so KB ZONE LEDs illuminate
        handle_baked_button_click(
            &assets.layout.buttons[30],
            false,
            &mut state,
            &audio,
            &assets,
            &mut leds,
            &mut tris,
        );
        assert!(state.program_ui.split.on, "SPLIT button must enable split");
        sync_baked_leds(&state, &assets, &mut leds);
        assert!(
            ORGAN_KB_ZONE_LEDS.iter().all(|&i| leds[i].0),
            "all 4 Organ KB ZONE LEDs must light when split is on (default all-zones)"
        );

        // Organ OCTAVE UP (btn 8), Piano OCTAVE DOWN (btn 20), Synth OCTAVE UP (btn 61)
        handle_baked_button_click(
            &assets.layout.buttons[8],
            false,
            &mut state,
            &audio,
            &assets,
            &mut leds,
            &mut tris,
        );
        assert_eq!(state.organ_octave_of(0), 1);

        handle_baked_button_click(
            &assets.layout.buttons[20],
            false,
            &mut state,
            &audio,
            &assets,
            &mut leds,
            &mut tris,
        );
        assert_eq!(state.piano_ui.octave[0], -1);

        handle_baked_button_click(
            &assets.layout.buttons[61],
            false,
            &mut state,
            &audio,
            &assets,
            &mut leds,
            &mut tris,
        );
        assert_eq!(state.synth_ui.layer().octave, 1);

        // Shift+OCTAVE UP steps KB ZONE from all-zones (0b1111) to zone 1 (0b0001)
        handle_baked_button_click(
            &assets.layout.buttons[8],
            true,
            &mut state,
            &audio,
            &assets,
            &mut leds,
            &mut tris,
        );
        sync_baked_leds(&state, &assets, &mut leds);
        assert_eq!(
            [
                leds[ORGAN_KB_ZONE_LEDS[0]].0,
                leds[ORGAN_KB_ZONE_LEDS[1]].0,
                leds[ORGAN_KB_ZONE_LEDS[2]].0,
                leds[ORGAN_KB_ZONE_LEDS[3]].0,
            ],
            [true, false, false, false],
            "Shift+Organ OCTAVE+ must light only KB ZONE LED 1"
        );

        let cmds: Vec<_> = rx.try_iter().collect();
        assert!(cmds.len() >= 5, "expected real AudioCommands, got {cmds:?}");
    }

    #[test]
    fn all_97_baked_buttons_dispatch_safely_against_live_engine() {
        let ctx = egui::Context::default();
        let assets = BakedPanelAssets::get_or_load(&ctx);
        assert_eq!(assets.layout.buttons.len(), 97);

        for b in &assets.layout.buttons {
            let mut state = Stage4PanelState::default();
            let (audio, rx) = AudioHandle::new();
            let mut leds: Vec<(bool, String)> = assets
                .layout
                .leds
                .iter()
                .map(|l| (l.on, l.color.clone()))
                .collect();
            let mut tris: Vec<bool> = assets.layout.tris.iter().map(|t| t.on).collect();

            handle_baked_button_click(b, false, &mut state, &audio, &assets, &mut leds, &mut tris);
            sync_baked_leds(&state, &assets, &mut leds);

            let mut engine = nord_core::engine::Stage4Engine::new(48_000.0);
            let mut vol = 0.85;
            for cmd in rx.try_iter() {
                crate::audio::apply_audio_command(cmd, &mut engine, &mut vol);
            }
            let mut l = [0.0_f32; 32];
            let mut r = [0.0_f32; 32];
            engine.process_block(&mut l, &mut r);
            assert!(
                l.iter().chain(r.iter()).all(|s| s.is_finite()),
                "button {} produced non-finite audio",
                b.index
            );
        }
    }
}
