//! 1:1 Baked HTML/CSS Hardware Panel View (`panel_plate_2x.png` + `panel_layout.json`).
//!
//! Renders the exact offline-baked `nd matrix 1` deck with interactive sprites, LEDs,
//! live OLEDs, and an 88-key keybed wired to `AudioHandle` / `Stage4PanelState`.

use super::prelude::*;
use crate::baked::BakedPanelAssets;
use egui::{pos2, vec2, Align2, Color32, FontId, Id, Rect, Sense, Ui};

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
    let scale_x = plate_rect.width() / cw;

    let p = ui.painter().clone();
    assets.paint_plate(&p, plate_rect);

    // 1. Pitch Stick & Mod Wheel
    if let Some(r_pitch) = assets.layout.wheels.get("pitch_stick") {
        let rect = r_pitch.in_rect(plate_rect);
        let resp = ui.interact(rect, ui.id().with("baked_pitch"), Sense::click_and_drag());
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
        assets.paint_sprite_rotated(
            &p,
            "pitch_stick",
            center,
            vec2(38.0 * scale_x, 26.0 * scale_x),
            -0.174,
            Color32::WHITE,
        );
    }

    if let Some(r_mod) = assets.layout.wheels.get("mod_wheel") {
        let rect = r_mod.in_rect(plate_rect);
        let resp = ui.interact(rect, ui.id().with("baked_mod"), Sense::click_and_drag());
        if resp.dragged() {
            state.mod_wheel = (state.mod_wheel - resp.drag_delta().y * 0.012).clamp(0.0, 1.0);
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
        let drum_size = vec2(22.0 * scale_x, 94.0 * scale_x);
        assets.paint_sprite_rotated(
            &p,
            "mod_wheel_drum",
            rect.center(),
            drum_size,
            mod_tilt,
            Color32::WHITE,
        );
        // Draw the moving grip dimple/groove along the tilted drum axis so rotation is visible.
        let axis = vec2(-mod_tilt.sin(), mod_tilt.cos());
        let along = (0.5 - state.mod_wheel) * drum_size.y * 0.62;
        let groove_center = rect.center() + axis * along;
        let (sin, cos) = mod_tilt.sin_cos();
        let (gw, gh) = (drum_size.x * 0.64 * 0.5, 4.5 * scale_x * 0.5);
        let pts = [(-1.0_f32, -1.0_f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, y)| {
                let (px, py) = (x * gw, y * gh);
                groove_center + vec2(px * cos - py * sin, px * sin + py * cos)
            })
            .to_vec();
        p.add(egui::Shape::convex_polygon(
            pts,
            Color32::from_black_alpha(150),
            egui::Stroke::new(0.9 * scale_x, Color32::from_white_alpha(120)),
        ));
    }

    // 2. Knobs & Encoders
    let knob_vals_id = Id::new("ns4_baked_knob_vals");
    for k in &assets.layout.knobs {
        let rect = norm_rect(k.x, k.y, k.w, k.h, plate_rect);
        let hit_size = if k.encoder { 28.0 } else { 30.0 } * scale_x;
        let hit_rect = Rect::from_center_size(rect.center(), vec2(hit_size, hit_size));
        let resp = ui.interact(
            hit_rect,
            ui.id().with(("bk", k.index)),
            Sense::click_and_drag(),
        );
        let mut val: f32 = ui.ctx().data(|d| {
            d.get_temp::<f32>(knob_vals_id.with(k.index))
                .unwrap_or(if k.index == 0 {
                    state.master_volume
                } else {
                    k.default_value
                })
        });
        if resp.dragged() {
            let delta = -resp.drag_delta().y * 0.007;
            val = if k.encoder {
                val + delta
            } else {
                (val + delta).clamp(0.0, 1.0)
            };
            ui.ctx()
                .data_mut(|d| d.insert_temp(knob_vals_id.with(k.index), val));
            if k.index == 0 {
                state.master_volume = val;
                audio_handle.send_command(AudioCommand::SetMasterVolume(val));
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
    }

    // Load / initialize live LED states from the HTML/CSS prototype defaults
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

    // 3. Pill Buttons (wired 1:1 to HTML/JS LED + Program behaviors)
    for b in &assets.layout.buttons {
        let rect = norm_rect(b.x, b.y, b.w, b.h, plate_rect);
        let resp = ui.interact(rect, ui.id().with(("bb", b.index)), Sense::click());
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
        let resp = ui.interact(rect, ui.id().with(("bf", idx)), Sense::click_and_drag());
        let level_ref: &mut f32 = match idx {
            0 => &mut state.organ_a_level,
            1 => &mut state.organ_b_level,
            2 => &mut state.piano_a_level,
            3 => &mut state.piano_b_level,
            4 => &mut state.synth_a_level,
            5 => &mut state.synth_b_level,
            _ => &mut state.synth_c_level,
        };
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                *level_ref = (1.0 - (pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
            }
        }
        let val = *level_ref;
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
        let resp = ui.interact(rect, ui.id().with(("bd", idx)), Sense::click_and_drag());
        let slot = idx.min(8);
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let n = ((pos.y - rect.top() - 12.0 * scale_x) / (rect.height() * 0.65))
                    .clamp(0.0, 1.0);
                let step = (n * 8.0).round() as u8;
                if step != state.organ_drawbars[slot] {
                    state.organ_drawbars[slot] = step;
                }
            }
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
            p.circle_filled(r.center(), radius, col);
            p.circle_stroke(
                r.center(),
                radius * 1.65,
                egui::Stroke::new(1.3, col.gamma_multiply(0.35)),
            );
        } else {
            p.circle_filled(r.center(), radius * 0.85, Color32::from_rgb(24, 20, 22));
        }
    }

    // 7. Live OLED Displays
    if let Some(main_oled) = assets.layout.oleds.get("main") {
        let r = main_oled.in_rect(plate_rect);
        p.rect_filled(r, 2.0, PROTOTYPE_OLED_BG);
        p.rect_stroke(
            r,
            2.0,
            egui::Stroke::new(1.0, Color32::from_rgb(24, 28, 34)),
            egui::StrokeKind::Inside,
        );
        let pad_x = r.width() * 0.06;
        let font_sm = (r.height() * 0.19).clamp(7.0, 13.0);
        let font_lg = (r.height() * 0.28).clamp(9.0, 18.0);
        let title = if state.prog_title.is_empty() {
            "nd matrix 1"
        } else {
            &state.prog_title
        };
        p.text(
            pos2(r.left() + pad_x, r.top() + r.height() * 0.10),
            Align2::LEFT_TOP,
            &state.prog_id,
            FontId::monospace(font_sm),
            PROTOTYPE_OLED_FG,
        );
        let layers = format!(
            "{}{}{}",
            if state.organ_a_active || state.organ_b_active {
                "ORG "
            } else {
                ""
            },
            if state.piano_a_active || state.piano_b_active {
                "PNO "
            } else {
                ""
            },
            if state.synth_a_active || state.synth_b_active || state.synth_c_active {
                "SYN"
            } else {
                ""
            }
        );
        p.text(
            pos2(r.right() - pad_x, r.top() + r.height() * 0.10),
            Align2::RIGHT_TOP,
            layers.trim(),
            FontId::monospace(font_sm * 0.88),
            Color32::from_rgb(0x8E, 0xB8, 0xD8),
        );
        p.text(
            pos2(r.left() + pad_x, r.top() + r.height() * 0.36),
            Align2::LEFT_TOP,
            title,
            FontId::monospace(font_lg),
            PROTOTYPE_OLED_FG,
        );
        let sub = format!(
            "MST {:.0}%  MOD {:.0}%",
            state.master_volume * 100.0,
            state.mod_wheel * 100.0
        );
        p.text(
            pos2(r.left() + pad_x, r.bottom() - r.height() * 0.12),
            Align2::LEFT_BOTTOM,
            sub,
            FontId::monospace(font_sm * 0.88),
            Color32::from_rgb(0x9A, 0xC0, 0xDF),
        );
    }
    if let Some(synth_oled) = assets.layout.oleds.get("synth") {
        let r = synth_oled.in_rect(plate_rect);
        p.rect_filled(r, 2.0, PROTOTYPE_OLED_BG);
        p.rect_stroke(
            r,
            2.0,
            egui::Stroke::new(1.0, Color32::from_rgb(24, 28, 34)),
            egui::StrokeKind::Inside,
        );
        let bar_h = (r.height() * 0.24).max(9.0);
        let bar_rect = Rect::from_min_size(
            pos2(r.left() + 2.0, r.top() + 2.0),
            vec2((r.width() - 4.0).max(4.0), bar_h),
        );
        p.rect_filled(bar_rect, 1.0, Color32::from_rgb(0x9A, 0xC0, 0xDF));
        let font_bar = (bar_h * 0.72).clamp(6.5, 11.0);
        let mode_title = match state.synth_osc_mode {
            SynthOscMode::Sample => "SAMPLE",
            SynthOscMode::Analog => "OSC WAVEFORM",
            _ => "SYNTH OSC",
        };
        p.text(
            bar_rect.center(),
            Align2::CENTER_CENTER,
            mode_title,
            FontId::monospace(font_bar),
            Color32::from_rgb(7, 9, 13),
        );
        let label = match state.synth_osc_mode {
            SynthOscMode::Sample => state
                .loaded_sample_name
                .as_deref()
                .unwrap_or("Sample")
                .to_string(),
            _ => format!("{:?}", state.synth_waveform),
        };
        let font_body = (r.height() * 0.22).clamp(7.5, 14.0);
        p.text(
            pos2(
                r.left() + r.width() * 0.06,
                bar_rect.bottom() + r.height() * 0.06,
            ),
            Align2::LEFT_TOP,
            label,
            FontId::monospace(font_body),
            PROTOTYPE_OLED_FG,
        );
        // Simple crisp waveform trace in lower third
        let wf_top = r.top() + r.height() * 0.62;
        let wf_bot = r.bottom() - r.height() * 0.12;
        let wf_mid = (wf_top + wf_bot) * 0.5;
        let wf_amp = (wf_bot - wf_top) * 0.45;
        let x0 = r.left() + r.width() * 0.08;
        let x1 = r.right() - r.width() * 0.08;
        let steps = 28;
        let mut pts = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = egui::lerp(x0..=x1, t);
            let phase = (t * 2.0) % 1.0;
            let y_norm = match state.synth_waveform {
                VaWaveform::Pulse => {
                    if phase < 0.5 {
                        -1.0
                    } else {
                        1.0
                    }
                }
                VaWaveform::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                VaWaveform::Sine => (phase * std::f32::consts::TAU).sin(),
                VaWaveform::Saw | VaWaveform::SuperSaw => 2.0 * phase - 1.0,
            };
            pts.push(pos2(x, wf_mid + y_norm * wf_amp));
        }
        p.add(egui::Shape::line(
            pts,
            egui::Stroke::new(1.2, PROTOTYPE_OLED_FG),
        ));
    }

    // 8. Interactive 88-Key Keybed (single hit-box so drag glissando & black-key priority work)
    let mut keys_bbox = Rect::NOTHING;
    for key in &assets.layout.keys {
        keys_bbox = keys_bbox.union(norm_rect(key.x, key.y, key.w, key.h, plate_rect));
    }
    if keys_bbox.is_positive() {
        let resp = ui.interact(
            keys_bbox,
            ui.id().with("stage4_baked_keybed"),
            Sense::click_and_drag(),
        );
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
    // Render pressed keys matching HTML/CSS .key-white.active & .key-black.active
    // White keys first, then black keys on top so black keys are never covered.
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
            if !assets.paint_sprite(&p, "key_white_down", r, Color32::WHITE) {
                p.rect_filled(r, 2.0, Color32::from_rgb(218, 216, 208));
            }
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
            if !assets.paint_sprite(&p, "key_black_down", r, Color32::WHITE) {
                p.rect_filled(r, 2.0, Color32::from_rgb(48, 52, 60));
            }
        } else if overlaps_pressed_white {
            assets.paint_sprite(&p, "key_black_up", r, Color32::WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioHandle;

    #[test]
    fn baked_panel_renders_all_layers_without_panics() {
        let mut state = Stage4PanelState {
            use_baked_plate: true,
            ..Default::default()
        };
        let (audio, _rx) = AudioHandle::new();
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render_baked_panel(ui, &mut state, &audio);
            });
        });
    }
}
