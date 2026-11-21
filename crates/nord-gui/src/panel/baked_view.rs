//! 1:1 Baked HTML/CSS Hardware Panel View (`panel_plate_2x.png` + `panel_layout.json`).
//!
//! Renders the exact offline-baked `nd matrix 1` deck with interactive sprites, LEDs,
//! live OLEDs, and an 88-key keybed wired to `AudioHandle` / `Stage4PanelState`.

use super::prelude::*;
use crate::baked::BakedPanelAssets;
use egui::{pos2, vec2, Color32, Id, Rect, Sense, Ui};

fn norm_rect(x: f32, y: f32, w: f32, h: f32, container: Rect) -> Rect {
    Rect::from_min_size(
        pos2(
            container.left() + x * container.width(),
            container.top() + y * container.height(),
        ),
        vec2(w * container.width(), h * container.height()),
    )
}

/// Draw the 1:1 baked hardware panel fitted to `ui.available_size()`.
pub fn render_baked_panel(ui: &mut Ui, state: &mut Stage4PanelState, audio_handle: &AudioHandle) {
    let assets = BakedPanelAssets::get_or_load(ui.ctx());
    let cw = assets.layout.chassis_px.width.max(1) as f32;
    let ch = assets.layout.chassis_px.height.max(1) as f32;
    let aspect = cw / ch;

    let avail = ui.available_size();
    let fit_w = avail.x.min(avail.y * aspect).max(320.0);
    let fit_h = fit_w / aspect;
    let (alloc, _) = ui.allocate_exact_size(vec2(avail.x.max(fit_w), fit_h), Sense::hover());
    let plate_rect = Rect::from_center_size(alloc.center(), vec2(fit_w, fit_h));
    ui.ctx()
        .data_mut(|d| d.insert_temp(Id::new("ns4_baked_plate_rect"), plate_rect));
    let scale_x = plate_rect.width() / cw;

    let p = ui.painter().clone();
    assets.paint_plate(&p, plate_rect);

    // 1. Pitch Stick & Mod Wheel
    if let Some(r_pitch) = assets.layout.wheels.get("pitch_stick") {
        let rect = r_pitch.in_rect(plate_rect);
        let resp = ui
            .interact(rect, ui.id().with("baked_pitch"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if resp.dragged() {
            state.pitch_stick = (state.pitch_stick + resp.drag_delta().x * 0.025).clamp(-1.0, 1.0);
            let pb = ((state.pitch_stick + 1.0) * 8192.0).round() as u16;
            audio_handle.send_keyboard_midi(MidiEvent::PitchBend {
                channel: 0,
                value: pb.min(16383),
            });
        } else if !resp.is_pointer_button_down_on() && state.pitch_stick.abs() > 1e-4 {
            state.pitch_stick *= 0.5;
            if state.pitch_stick.abs() < 1e-3 {
                state.pitch_stick = 0.0;
            }
        }
        let travel = rect.width() * 0.22 * state.pitch_stick;
        let center = rect.center() + vec2(travel, -travel * 0.17);
        // .pitch-stick-wood is 34x22 px + 14 px sprite padding on each side = 62x50 px.
        assets.paint_sprite_rotated(
            &p,
            "pitch_stick",
            center,
            vec2(62.0 * scale_x, 50.0 * scale_x),
            -0.174,
            Color32::WHITE,
        );
    }

    if let Some(r_mod) = assets.layout.wheels.get("mod_wheel") {
        let rect = r_mod.in_rect(plate_rect);
        let resp = ui
            .interact(rect, ui.id().with("baked_mod"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        let scroll_dy = if resp.hovered() {
            ui.input(|i| i.smooth_scroll_delta.y)
        } else {
            0.0
        };
        if resp.dragged() || scroll_dy.abs() > 0.1 {
            let delta = if resp.dragged() {
                -resp.drag_delta().y * 0.012
            } else {
                scroll_dy * 0.0025
            };
            state.mod_wheel = (state.mod_wheel + delta).clamp(0.0, 1.0);
            let val = (state.mod_wheel * 127.0).round() as u8;
            audio_handle.send_keyboard_midi(MidiEvent::ControlChange {
                channel: 0,
                controller: 1,
                value: val,
            });
        } else if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                state.mod_wheel =
                    (1.0 - (pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
                let val = (state.mod_wheel * 127.0).round() as u8;
                audio_handle.send_keyboard_midi(MidiEvent::ControlChange {
                    channel: 0,
                    controller: 1,
                    value: val,
                });
            }
        }
        let mod_tilt: f32 = -0.174;
        // .mod-wheel-drum is 15x86 px + 14 px sprite padding on each side = 43x114 px.
        assets.paint_sprite_rotated(
            &p,
            "mod_wheel_drum",
            rect.center(),
            vec2(43.0 * scale_x, 114.0 * scale_x),
            mod_tilt,
            Color32::WHITE,
        );
        // Rolling stone-wheel ribs + recessed dimple along the tilted drum axis so the wheel
        // visibly rolls just like background-position in panel.js.
        let drum_h = 86.0 * scale_x;
        let drum_w = 13.0 * scale_x;
        let axis = vec2(-mod_tilt.sin(), mod_tilt.cos());
        let (sin, cos) = mod_tilt.sin_cos();
        let rib_pitch = 9.0 * scale_x;
        let phase = (-state.mod_wheel * 36.0 * scale_x).rem_euclid(rib_pitch);
        let half_h = drum_h * 0.44;
        let mut y_off = -half_h + phase;
        while y_off <= half_h {
            let t = y_off / half_h;
            let alpha = ((1.0 - t * t).max(0.0) * 42.0) as u8;
            let rc = rect.center() + axis * y_off;
            let hw = drum_w * 0.42;
            p.line_segment(
                [rc - vec2(hw * cos, hw * sin), rc + vec2(hw * cos, hw * sin)],
                egui::Stroke::new(1.0 * scale_x, Color32::from_black_alpha(alpha)),
            );
            y_off += rib_pitch;
        }
        let along = (0.24 - state.mod_wheel * 0.62) * drum_h;
        let dimple_center = rect.center() + axis * along;
        let rx = 5.0 * scale_x;
        let ry = 5.5 * scale_x;
        p.circle_filled(
            dimple_center + vec2(0.0, 0.8 * scale_x),
            rx.max(ry),
            Color32::from_white_alpha(75),
        );
        p.circle_filled(
            dimple_center,
            rx.max(ry) * 0.92,
            Color32::from_rgba_unmultiplied(36, 40, 46, 210),
        );
        p.circle_filled(
            dimple_center + vec2(0.8 * scale_x, -0.6 * scale_x),
            rx * 0.55,
            Color32::from_white_alpha(60),
        );
    }

    // 2. Knobs & Encoders
    let knob_vals_id = Id::new("ns4_baked_knob_vals");
    for k in &assets.layout.knobs {
        let rect = norm_rect(k.x, k.y, k.w, k.h, plate_rect);
        let hit_size = if k.encoder { 28.0 } else { 30.0 } * scale_x;
        let hit_rect = Rect::from_center_size(rect.center(), vec2(hit_size, hit_size));
        let resp = ui
            .interact(
                hit_rect,
                ui.id().with(("bk", k.index)),
                Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        let mut val: f32 = ui.ctx().data(|d| {
            d.get_temp::<f32>(knob_vals_id.with(k.index))
                .unwrap_or(if k.index == 0 {
                    state.master_volume
                } else {
                    k.default_value
                })
        });
        let scroll_dy = if resp.hovered() {
            ui.input(|i| i.smooth_scroll_delta.y)
        } else {
            0.0
        };
        if resp.dragged() || scroll_dy.abs() > 0.1 {
            let delta = if resp.dragged() {
                -resp.drag_delta().y * 0.007
            } else {
                scroll_dy * 0.0018
            };
            val = if k.encoder {
                val + delta
            } else {
                (val + delta).clamp(0.0, 1.0)
            };
            ui.ctx()
                .data_mut(|d| d.insert_temp(knob_vals_id.with(k.index), val));
            match k.index {
                0 => {
                    state.master_volume = val;
                    audio_handle.send_command(AudioCommand::SetMasterVolume(val));
                }
                1 => {
                    state.rotary_drive = val;
                    audio_handle.send_command(AudioCommand::SetRotaryDrive(val));
                }
                _ => {}
            }
        }
        let angle = if k.encoder {
            val * std::f32::consts::TAU
        } else {
            (-135.0 + val * 270.0).to_radians()
        };
        let (sprite, sprite_px) = if k.encoder {
            ("knob_encoder_cap", 54.0)
        } else {
            ("knob_pot_cap", 56.0)
        };
        let size = vec2(sprite_px * scale_x, sprite_px * scale_x);
        let tint = if resp.hovered() || resp.dragged() {
            Color32::WHITE
        } else {
            Color32::from_gray(244)
        };
        // Fixed top-left light -> bottom-right drop shadow (never rotates with the knob cap).
        let shadow_r = (if k.encoder { 13.0 } else { 14.0 }) * scale_x;
        p.circle_filled(
            rect.center() + vec2(0.0, 2.8 * scale_x),
            shadow_r * 1.05,
            Color32::from_black_alpha(60),
        );
        p.circle_filled(
            rect.center() + vec2(0.0, 1.8 * scale_x),
            shadow_r * 0.96,
            Color32::from_black_alpha(110),
        );
        assets.paint_sprite_rotated(&p, sprite, rect.center(), size, angle, tint);
        if k.encoder {
            let dot_r = 8.0 * scale_x;
            let (s, c) = (angle - std::f32::consts::FRAC_PI_2).sin_cos();
            p.circle_filled(
                rect.center() + vec2(c * dot_r, s * dot_r),
                1.6 * scale_x,
                Color32::from_rgba_unmultiplied(215, 224, 234, 190),
            );
        }
    }

    // Load / initialize live LED states and triangular matrix states from HTML/CSS defaults
    let leds_id = Id::new("ns4_baked_leds");
    let mut led_states: Vec<(bool, String)> = ui.ctx().data(|d| {
        d.get_temp::<Vec<(bool, String)>>(leds_id)
            .filter(|v| v.len() == assets.layout.leds.len())
            .unwrap_or_else(|| {
                assets
                    .layout
                    .leds
                    .iter()
                    .map(|l| (l.on, l.color.clone()))
                    .collect()
            })
    });
    let tris_id = Id::new("ns4_baked_tris");
    let mut tri_states: Vec<bool> = ui.ctx().data(|d| {
        d.get_temp::<Vec<bool>>(tris_id)
            .filter(|v| v.len() == assets.layout.tris.len())
            .unwrap_or_else(|| assets.layout.tris.iter().map(|t| t.on).collect())
    });

    // 3. Pill Buttons (wired 1:1 to HTML/JS LED + Program behaviors)
    for b in &assets.layout.buttons {
        let rect = norm_rect(b.x, b.y, b.w, b.h, plate_rect);
        let resp = ui
            .interact(rect, ui.id().with(("bb", b.index)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let down = resp.is_pointer_button_down_on();
        if resp.clicked() {
            let find_led = |id: &str| {
                assets
                    .layout
                    .leds
                    .iter()
                    .position(|l| l.id.as_deref() == Some(id))
            };
            if let Some(id) = b.toggle_led.as_deref() {
                if let Some(li) = find_led(id) {
                    led_states[li].0 = !led_states[li].0;
                    let on = led_states[li].0;
                    match id {
                        "org-a" => state.organ_a_active = on,
                        "org-b" => state.organ_b_active = on,
                        "pno-a" => state.piano_a_active = on,
                        "pno-b" => state.piano_b_active = on,
                        "syn-a" => state.synth_a_active = on,
                        "syn-b" => state.synth_b_active = on,
                        "syn-c" => state.synth_c_active = on,
                        "m-whl" => state.morph_wheel_active = on,
                        "m-at" => state.morph_at_active = on,
                        "m-ped" => state.morph_ctrl_active = on,
                        "dly-on" => state.delay_on = on,
                        "rotary-stop" => state.rotary_stop = on,
                        _ => {}
                    }
                }
            }
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
                            audio_handle
                                .send_command(AudioCommand::SetRotarySpeed(state.rotary_speed));
                        }
                    }
                }
            }
            if let Some(cycle) = b.cycle_leds.as_deref() {
                let indices: Vec<usize> = cycle.split(',').filter_map(find_led).collect();
                if !indices.is_empty() {
                    let cur = indices.iter().position(|&li| led_states[li].0);
                    let mut next = cur.map_or(0, |c| c + 1);
                    if next >= indices.len() {
                        if b.allow_off {
                            next = usize::MAX;
                        } else {
                            next = 0;
                        }
                    }
                    for (pos, &li) in indices.iter().enumerate() {
                        led_states[li].0 = pos == next;
                    }
                }
            }
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
                    }
                }
            }
            if let Some(p_num) = b.prog_btn {
                for (li, l) in assets.layout.leds.iter().enumerate() {
                    if let Some(pl) = l.prog_led {
                        led_states[li].0 = pl == p_num;
                    }
                }
                let (pid, ptitle) = match p_num {
                    1 => ("A:11", "nd matrix 1"),
                    2 => ("A:12", "Royal Grand 3D"),
                    3 => ("A:13", "B3 Gospel Swell"),
                    4 => ("A:14", "MK1 Suitcase EP"),
                    5 => ("A:15", "Super Saw Lead"),
                    6 => ("A:16", "Horizon Whl"),
                    7 => ("A:17", "Warm Analog Pad"),
                    _ => ("A:18", "Stage Split Live"),
                };
                state.prog_id = pid.to_string();
                state.prog_title = ptitle.to_string();
            }
            if b.split_btn {
                let split_indices: Vec<usize> = assets
                    .layout
                    .leds
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.split)
                    .map(|(i, _)| i)
                    .collect();
                let cur_count = split_indices.iter().filter(|&&i| led_states[i].0).count();
                let next_count = (cur_count + 1) % 4;
                for (pos, &li) in split_indices.iter().enumerate() {
                    led_states[li].0 = pos < next_count;
                }
            }
            if let Some(target) = b.fx_focus.as_deref() {
                let groups: [(&str, &str, &[&str], &str); 3] = [
                    ("organ", "org-fx-focus", &["fxf-org-a", "fxf-org-b"], "red"),
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
                ui.ctx()
                    .data_mut(|d| d.insert_temp(tris_id, tri_states.clone()));
            }
            ui.ctx()
                .data_mut(|d| d.insert_temp(leds_id, led_states.clone()));
        }
        let sprite = match (b.variant.as_str(), down) {
            ("grey", true) => "btn_grey_down",
            ("grey", false) => "btn_grey_up",
            ("red", true) => "btn_red_down",
            ("red", false) => "btn_red_up",
            (_, true) => "btn_black_down",
            (_, false) => "btn_black_up",
        };
        let tint = if resp.hovered() && !down {
            Color32::WHITE
        } else {
            Color32::from_gray(242)
        };
        if b.vertical {
            assets.paint_sprite_rotated(
                &p,
                sprite,
                rect.center(),
                vec2(rect.height() * 1.45, rect.width() * 1.75),
                std::f32::consts::FRAC_PI_2,
                tint,
            );
        } else {
            assets.paint_sprite(&p, sprite, rect.expand(4.0 * scale_x), tint);
        }
    }

    // 4. 7 Layer Faders + 11-LED Green Ladders
    for (idx, f) in assets.layout.faders.iter().enumerate() {
        let rect = f.in_rect(plate_rect);
        let resp = ui
            .interact(rect, ui.id().with(("bf", idx)), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        let level_ref: &mut f32 = match idx {
            0 => &mut state.organ_a_level,
            1 => &mut state.organ_b_level,
            2 => &mut state.piano_a_level,
            3 => &mut state.piano_b_level,
            4 => &mut state.synth_a_level,
            5 => &mut state.synth_b_level,
            _ => &mut state.synth_c_level,
        };
        let scroll_dy = if resp.hovered() {
            ui.input(|i| i.smooth_scroll_delta.y)
        } else {
            0.0
        };
        let mut fader_changed = false;
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                *level_ref = (1.0 - (pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
                fader_changed = true;
            }
        } else if scroll_dy.abs() > 0.1 {
            *level_ref = (*level_ref + scroll_dy * 0.002).clamp(0.0, 1.0);
            fader_changed = true;
        }
        let val = *level_ref;
        if fader_changed {
            let cmd = match idx {
                0 => AudioCommand::SetOrganLevel(val),
                1 => AudioCommand::SetOrganBLevel(val),
                2 => AudioCommand::SetPianoLevel(val),
                3 => AudioCommand::SetPianoBLevel(val),
                4 => AudioCommand::SetSynthLevel(val),
                5 => AudioCommand::SetSynthBLevel(val),
                _ => AudioCommand::SetSynthCLevel(val),
            };
            audio_handle.send_command(cmd);
        }
        let cy = rect.top() + (0.10 + (1.0 - val) * 0.80) * rect.height();
        let cap = Rect::from_center_size(
            pos2(rect.center().x, cy),
            vec2(28.0 * scale_x, 22.0 * scale_x),
        );
        assets.paint_sprite(&p, "fader_cap", cap, Color32::WHITE);

        // 11-segment green LED ladder using exact DOM rects from panel_layout.json
        let lit_segs = (val * 11.0).round() as usize;
        if let Some(segs) = assets.layout.fader_ladders.get(idx) {
            let mut sorted: Vec<Rect> = segs.iter().map(|s| s.in_rect(plate_rect)).collect();
            sorted.sort_by(|a, b| b.center().y.total_cmp(&a.center().y));
            for (seg_idx, sr) in sorted.into_iter().enumerate() {
                let on = seg_idx < lit_segs && val > 0.01;
                let col = if on {
                    LED_GREEN
                } else {
                    Color32::from_rgb(22, 30, 24)
                };
                p.rect_filled(sr, 0.8, col);
            }
        }
    }

    // 5. 9 Organ Drawbars + 8-LED Red Ladders
    let drawbar_white = [false, false, true, true, false, true, false, false, true];
    for (idx, d) in assets.layout.drawbars.iter().enumerate() {
        let rect = d.in_rect(plate_rect);
        let resp = ui
            .interact(rect, ui.id().with(("bd", idx)), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        let slot = idx.min(8);
        let scroll_dy = if resp.hovered() {
            ui.input(|i| i.smooth_scroll_delta.y)
        } else {
            0.0
        };
        let mut drawbar_changed = false;
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let n = ((pos.y - rect.top() - 12.0 * scale_x) / (rect.height() * 0.65))
                    .clamp(0.0, 1.0);
                let step = (n * 8.0).round() as u8;
                if step != state.organ_drawbars[slot] {
                    state.organ_drawbars[slot] = step;
                    drawbar_changed = true;
                }
            }
        } else if scroll_dy.abs() > 0.1 {
            let cur = state.organ_drawbars[slot] as i8;
            let delta = if scroll_dy > 0.0 { -1 } else { 1 };
            let next = (cur + delta).clamp(0, 8) as u8;
            if next != state.organ_drawbars[slot] {
                state.organ_drawbars[slot] = next;
                drawbar_changed = true;
            }
        }
        if drawbar_changed {
            audio_handle.send_command(AudioCommand::SetDrawbars(state.organ_drawbars));
        }
        let step_u8 = state.organ_drawbars[slot];
        let step = step_u8 as f32 / 8.0;
        let cy = rect.top() + (0.20 + step * 0.56) * rect.height();
        let cap = Rect::from_center_size(
            pos2(rect.center().x, cy),
            vec2(26.0 * scale_x, 62.0 * scale_x),
        );
        let sprite = if drawbar_white[slot] {
            "drawbar_white"
        } else {
            "drawbar_black"
        };
        assets.paint_sprite(&p, sprite, cap, Color32::WHITE);

        // 8-segment red LED ladder using exact DOM rects inside the black LED column
        if let Some(segs) = assets.layout.drawbar_ladders.get(idx) {
            let mut sorted: Vec<Rect> = segs.iter().map(|s| s.in_rect(plate_rect)).collect();
            sorted.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
            for (seg_idx, sr) in sorted.into_iter().enumerate() {
                let on = (seg_idx as u8) < step_u8;
                let col = if on {
                    LED_RED
                } else {
                    Color32::from_rgb(32, 18, 20)
                };
                p.rect_filled(sr, 0.8, col);
            }
        }
    }

    // 6. Active Hardware LEDs (live from ns4_baked_leds + MIDI activity)
    for (idx, led) in assets.layout.leds.iter().enumerate() {
        let (mut on, ref color_name) = led_states[idx];
        if led.id.as_deref() == Some("midi-led") && state.leds.midi_on() {
            on = true;
        }
        let r = norm_rect(led.x, led.y, led.w, led.h, plate_rect);
        let radius = (r.width().min(r.height()) * 0.46).max(2.0);
        if on {
            let col = match color_name.as_str() {
                "green" => LED_GREEN,
                "amber" => LED_AMBER,
                _ => LED_RED,
            };
            p.circle_filled(r.center(), radius * 2.1, col.gamma_multiply(0.14));
            p.circle_filled(r.center(), radius * 1.45, col.gamma_multiply(0.28));
            p.circle_filled(r.center(), radius, col);
            p.circle_filled(
                r.center() - vec2(radius * 0.25, radius * 0.25),
                (radius * 0.35).max(1.0),
                Color32::from_white_alpha(140),
            );
        } else {
            p.circle_filled(r.center(), radius * 0.85, Color32::from_rgb(24, 20, 22));
        }
    }

    // 6b. Triangular Matrix LEDs (ORGAN MODEL, VIB/CHORUS, PIANO SELECT, MOD 1/2, AMP, REVERB)
    for (idx, t) in assets.layout.tris.iter().enumerate() {
        let on = tri_states[idx];
        let r = norm_rect(t.x, t.y, t.w, t.h, plate_rect);
        let pts = match t.dir.as_str() {
            "left" => vec![r.left_center(), r.right_top(), r.right_bottom()],
            "right" => vec![r.right_center(), r.left_top(), r.left_bottom()],
            "up" => vec![r.center_top(), r.left_bottom(), r.right_bottom()],
            _ => vec![r.center_bottom(), r.left_top(), r.right_top()],
        };
        let fill = if on {
            LED_RED
        } else {
            Color32::from_rgb(32, 24, 26)
        };
        let stroke = if on {
            egui::Stroke::new(1.0, LED_RED.gamma_multiply(0.5))
        } else {
            egui::Stroke::NONE
        };
        p.add(egui::Shape::convex_polygon(pts, fill, stroke));
    }

    // 7. Live ARM-verified OLED Displays (reusing bay_program::oled & bay_synth::oled)
    if let Some(main_oled) = assets.layout.oleds.get("main") {
        let r = main_oled.in_rect(plate_rect);
        let summary = super::bay_program::oled::Summary {
            organ: format!("{:?}", state.organ_model),
            piano: state
                .loaded_piano_name
                .clone()
                .unwrap_or_else(|| format!("{:?}", state.piano_cat)),
            synth: state
                .loaded_sample_name
                .clone()
                .unwrap_or_else(|| format!("{:?}", state.synth_waveform)),
            piano_size: None,
            organ_bars: matches!(state.organ_model, OrganModel::B3).then_some(state.organ_drawbars),
            layers_on: [
                state.organ_a_active,
                state.organ_b_active,
                state.piano_a_active,
                state.piano_b_active,
                state.synth_a_active,
                state.synth_b_active,
                state.synth_c_active,
            ],
        };
        let mut oled_text = super::bay_program::oled::compose(&state.program_ui, &summary);
        if !state.prog_id.is_empty() && oled_text.bar.is_none() {
            oled_text.big = state.prog_id.clone();
        }
        if !state.prog_title.is_empty() && oled_text.bar.is_none() {
            oled_text.sub = state.prog_title.clone();
        }
        super::bay_program::oled::paint(&p, r, &oled_text, false);
    }
    if let Some(synth_oled) = assets.layout.oleds.get("synth") {
        let r = synth_oled.in_rect(plate_rect);
        super::bay_synth::oled::paint_oled_in_rect(
            &p,
            r,
            &mut state.synth_ui,
            state.loaded_sample_name.as_deref(),
            None,
        );
    }

    // 8. Interactive 88-Key Keybed (single hit-box so drag glissando & black-key priority work)
    let mut keys_bbox = Rect::NOTHING;
    for key in &assets.layout.keys {
        keys_bbox = keys_bbox.union(norm_rect(key.x, key.y, key.w, key.h, plate_rect));
    }
    if keys_bbox.is_positive() {
        let resp = ui
            .interact(
                keys_bbox,
                ui.id().with("stage4_baked_keybed"),
                Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.is_pointer_button_down_on() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let hit = assets
                    .layout
                    .keys
                    .iter()
                    .find(|k| k.black && norm_rect(k.x, k.y, k.w, k.h, plate_rect).contains(pos))
                    .or_else(|| {
                        assets.layout.keys.iter().find(|k| {
                            !k.black && norm_rect(k.x, k.y, k.w, k.h, plate_rect).contains(pos)
                        })
                    });
                let new_note = hit.map(|k| k.midi);
                if state.mouse_held_note != new_note {
                    if let Some(old) = state.mouse_held_note.take() {
                        audio_handle.send_keyboard_midi(MidiEvent::NoteOff {
                            channel: 0,
                            note: old,
                            velocity: 0,
                        });
                    }
                    if let (Some(n), Some(k)) = (new_note, hit) {
                        state.mouse_held_note = Some(n);
                        let kr = norm_rect(k.x, k.y, k.w, k.h, plate_rect);
                        let vel = super::keybed_velocity::velocity_at(
                            pos.y - kr.top(),
                            k.black,
                            kr.height(),
                        );
                        audio_handle.send_keyboard_midi(MidiEvent::NoteOn {
                            channel: 0,
                            note: n,
                            velocity: vel,
                        });
                        for c in state.program_ui.key_pressed(n) {
                            audio_handle.send_command(c);
                        }
                    }
                }
            }
        } else if let Some(old) = state.mouse_held_note.take() {
            audio_handle.send_keyboard_midi(MidiEvent::NoteOff {
                channel: 0,
                note: old,
                velocity: 0,
            });
        }
    }
    // Render pressed keys matching HTML/CSS .key-white.active & .key-black.active.
    // Clipped strictly to `keys_bbox` so pressed keys never bleed over the red felt lip, and
    // overlapping unpressed black keys are re-painted verbatim from `panel_plate_2x.png`.
    if keys_bbox.is_positive() {
        let key_p = p.with_clip_rect(keys_bbox);
        let shift_y = 2.0 * scale_x;
        let mut pressed_white_rects: Vec<Rect> = Vec::new();
        for key in &assets.layout.keys {
            if key.black {
                continue;
            }
            let is_pressed = state.midi_active_keys[key.midi as usize]
                || state.mouse_held_note == Some(key.midi)
                || state.computer_kb.is_sounding(key.midi);
            if is_pressed {
                let r = norm_rect(key.x, key.y, key.w, key.h, plate_rect);
                pressed_white_rects.push(r);
                let r_down = r.translate(vec2(0.0, shift_y));
                // Pressed ivory body (.key-white.active) + top depression shadow under felt lip
                key_p.rect_filled(r_down, 2.0 * scale_x, Color32::from_rgb(228, 226, 219));
                key_p.rect_filled(
                    Rect::from_min_size(r.left_top(), vec2(r.width(), 5.0 * scale_x)),
                    0.0,
                    Color32::from_black_alpha(55),
                );
                key_p.rect_stroke(
                    r_down,
                    2.0 * scale_x,
                    egui::Stroke::new(1.0, Color32::from_rgb(156, 154, 146)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        for key in &assets.layout.keys {
            if !key.black {
                continue;
            }
            let is_pressed = state.midi_active_keys[key.midi as usize]
                || state.mouse_held_note == Some(key.midi)
                || state.computer_kb.is_sounding(key.midi);
            let r = norm_rect(key.x, key.y, key.w, key.h, plate_rect);
            let overlaps_pressed_white = pressed_white_rects.iter().any(|wr| wr.intersects(r));
            if is_pressed {
                let r_down = r.translate(vec2(0.0, shift_y));
                key_p.rect_filled(r_down, 2.0 * scale_x, Color32::from_rgb(22, 24, 28));
                key_p.rect_filled(
                    r_down.shrink2(vec2(2.0 * scale_x, 3.0 * scale_x)),
                    1.5 * scale_x,
                    Color32::from_rgb(36, 40, 46),
                );
                key_p.rect_filled(
                    Rect::from_min_size(r.left_top(), vec2(r.width(), 4.0 * scale_x)),
                    0.0,
                    Color32::from_black_alpha(95),
                );
            } else if overlaps_pressed_white {
                assets.paint_plate_region(&key_p, plate_rect, key.x, key.y, key.w, key.h);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioHandle;
    use std::collections::BTreeSet;

    #[test]
    fn baked_panel_renders_all_layers_without_panics() {
        let mut state = Stage4PanelState {
            use_baked_plate: true,
            mouse_held_note: Some(60), // pressed white key C4
            pitch_stick: 0.45,
            mod_wheel: 0.65,
            ..Default::default()
        };
        state.midi_active_keys[61] = true; // pressed black key C#4
        let (audio, _rx) = AudioHandle::new();
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render_baked_panel(ui, &mut state, &audio);
            });
        });
    }

    #[test]
    fn baked_layout_contains_all_interactive_controls_ladders_and_tris() {
        let ctx = egui::Context::default();
        let assets = BakedPanelAssets::get_or_load(&ctx);
        let l = &assets.layout;
        assert!(
            l.knobs.len() >= 30,
            "expected >= 30 knobs, got {}",
            l.knobs.len()
        );
        assert!(
            l.buttons.len() >= 80,
            "expected >= 80 buttons, got {}",
            l.buttons.len()
        );
        assert_eq!(l.faders.len(), 7, "expected 7 layer faders");
        assert_eq!(l.fader_ladders.len(), 7, "expected 7 fader LED ladders");
        assert!(l.fader_ladders.iter().all(|segs| segs.len() == 11));
        assert_eq!(l.drawbars.len(), 9, "expected 9 organ drawbars");
        assert_eq!(l.drawbar_ladders.len(), 9, "expected 9 drawbar LED ladders");
        assert!(l.drawbar_ladders.iter().all(|segs| segs.len() == 8));
        assert_eq!(l.keys.len(), 88, "expected 88 keybed keys");
        assert!(l.oleds.contains_key("main") && l.oleds.contains_key("synth"));
        assert!(l.wheels.contains_key("pitch_stick") && l.wheels.contains_key("mod_wheel"));

        let tri_groups: BTreeSet<&str> = l.tris.iter().map(|t| t.group.as_str()).collect();
        assert_eq!(l.tris.len(), 42, "expected 42 triangular matrix LEDs");
        for expected in [
            "org-model",
            "org-vib",
            "pno-cat",
            "fx-mod1",
            "fx-mod2",
            "fx-amp",
            "fx-rev",
        ] {
            assert!(
                tri_groups.contains(expected),
                "missing triangular LED group {expected}"
            );
        }
    }
}
