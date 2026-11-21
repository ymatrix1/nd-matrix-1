//! Input flows of the Layer Effects bay: clicks, holds, drags and their engine edits.

use super::*;
use crate::audio::{FxEdit, FxFlag, FxFloat};
use crate::panel::ui_state::fx::Unit;
use egui::{Event, Modifiers};
use nord_core::fx::{AmpCabinet, Mod1Type, ReverbColor};

#[test]
fn mod1_on_then_selector_edits_the_organ_chains() {
    let mut s = Session::new();
    s.click("mod1.on");
    assert!(s.chain().mod1_on);
    assert!(s.fx_edits().contains(&FxEdit::Mod1Type {
        slot: 0,
        kind: Mod1Type::AutoPan
    }));
    s.click("mod1.selector");
    assert_eq!(s.chain().mod1_sel, 1);
    assert!(s.fx_edits().contains(&FxEdit::Mod1Type {
        slot: 0,
        kind: Mod1Type::Tremolo
    }));
    assert_eq!(s.state.mod1_type, Mod1Type::Tremolo, "legacy field follows");
}

#[test]
fn selector_cycles_through_a_wah_and_pump_and_the_engine_follows() {
    let mut s = Session::new();
    s.state.fx_ui.chains[0].mod1_on = true;
    s.state.fx_ui.chains[0].mod1_sel = 2; // RM
    s.click("mod1.selector");
    assert_eq!(s.chain().mod1_sel, 3, "A-WAH is next");
    assert!(s.fx_edits().contains(&FxEdit::Mod1Type {
        slot: 0,
        kind: Mod1Type::AutoWah
    }));
    s.click("mod1.selector");
    s.click("mod1.selector");
    assert_eq!(s.chain().mod1_sel, 5);
    assert!(s.fx_edits().contains(&FxEdit::Mod1Type {
        slot: 0,
        kind: Mod1Type::Pump
    }));
    s.click("mod1.selector");
    assert_eq!(s.chain().mod1_sel, 0, "wraps to A-PAN");
}

#[test]
fn mod2_selector_reaches_ens_and_spin() {
    let mut s = Session::new();
    s.state.fx_ui.chains[0].mod2_on = true;
    s.state.fx_ui.chains[0].mod2_sel = 3; // VIBE
    s.click("mod2.selector");
    assert!(s.fx_edits().contains(&FxEdit::Mod2Type {
        slot: 0,
        kind: nord_core::fx::Mod2Type::Ensemble
    }));
    s.click("mod2.selector");
    assert!(s.fx_edits().contains(&FxEdit::Mod2Type {
        slot: 0,
        kind: nord_core::fx::Mod2Type::Spin
    }));
}

#[test]
fn pump_shift_function_is_ped_and_a_wah_keeps_variation() {
    let mut s = Session::new();
    s.state.fx_ui.chains[0].mod1_sel = 5;
    s.shift_click("mod1.selector");
    assert!(s.chain().mod1_ped && !s.chain().mod1_variation);
    assert!(s.fx_edits().contains(&FxEdit::Flag {
        slot: 0,
        param: FxFlag::Mod1Ped,
        on: true
    }));
    s.state.fx_ui.chains[0].mod1_sel = 3;
    s.shift_click("mod1.selector");
    assert!(s.chain().mod1_variation, "A-WAH VARIATION");
}

#[test]
fn mod1_rate_knob_selects_the_master_clock_division_when_on() {
    let mut s = Session::new();
    s.state.fx_ui.chains[0].mod1_sel = 5; // PUMP
    s.state.fx_ui.chains[0].mod1_mst = 5; // MST CLK already on (1/4)
    s.drag_up("mod1.rate", 40.0);
    assert!(s.chain().mod1_mst > 5, "division {}", s.chain().mod1_mst);
    let want = crate::panel::ui_state::fx::mst_beats(s.chain().mod1_mst);
    assert!(s.fx_edits().contains(&FxEdit::Mod1Sync {
        slot: 0,
        beats: want
    }));
}

#[test]
fn mod1_mst_clk_is_inert_on_ring_mod() {
    let mut s = Session::new();
    s.state.fx_ui.chains[0].mod1_sel = 2; // RM
    s.state.fx_ui.chains[0].mod1_mst = 5;
    s.drag_up("mod1.rate", 40.0);
    assert!(s
        .fx_edits()
        .iter()
        .all(|e| !matches!(e, FxEdit::Mod1Sync { beats: Some(_), .. })));
    assert!(s.chain().mod1_rate > 4.0, "the knob edits the free rate");
}

#[test]
fn shift_click_and_hold_reach_the_shifted_function() {
    let mut s = Session::new();
    s.shift_click("mod2.selector");
    assert!(s.chain().mod2_variation);
    assert!(s.fx_edits().contains(&FxEdit::Flag {
        slot: 0,
        param: FxFlag::Mod2Variation,
        on: true
    }));
    s.click_for("mod2.selector", 0.7, Modifiers::NONE);
    assert!(!s.chain().mod2_variation, "hold-to-shift toggled it back");
    assert_eq!(s.chain().mod2_sel, 0, "hold must not also cycle the type");
}

#[test]
fn wah_shift_function_is_ped() {
    let mut s = Session::new();
    s.state.fx_ui.chains[0].mod1_sel = 4;
    s.shift_click("mod1.selector");
    assert!(s.chain().mod1_ped);
    assert!(!s.chain().mod1_variation);
    assert!(s.fx_edits().contains(&FxEdit::Flag {
        slot: 0,
        param: FxFlag::Mod1Ped,
        on: true
    }));
}

#[test]
fn knob_drag_edits_the_focused_chain_only() {
    let mut s = Session::new();
    s.drag_up("mod1.amount", 20.0);
    assert!(
        s.chain().mod1_amount > 0.45,
        "amount {}",
        s.chain().mod1_amount
    );
    let edits = s.fx_edits();
    assert!(edits.iter().any(|e| matches!(
        e,
        FxEdit::Float { slot: 0, param: FxFloat::Mod1Amount, value } if *value > 0.45
    )));
    assert!(edits.iter().all(|e| match e {
        FxEdit::Float { slot, .. } | FxEdit::Flag { slot, .. } => *slot == 0,
        _ => true,
    }));
}

#[test]
fn focus_buttons_move_the_bay_to_another_layer() {
    let mut s = Session::new();
    s.click("fx.focus.piano");
    assert_eq!(s.state.fx_focus, 1);
    s.click("fx.focus.piano");
    assert!(s.state.fx_ui.piano_b);
    s.click("mod2.on");
    assert!(s.state.fx_ui.chains[2].mod2_on, "edit landed on Piano B");
    assert!(!s.state.fx_ui.chains[0].mod2_on);
    assert!(s
        .fx_edits()
        .iter()
        .any(|e| matches!(e, FxEdit::Mod2Type { slot: 2, .. })));
}

#[test]
fn group_and_global_send_sharing_edits() {
    let mut s = Session::new();
    s.click("fx.focus.synth");
    s.shift_click("fx.focus.synth");
    let sharing: Vec<_> = s
        .fx_edits()
        .into_iter()
        .filter_map(|e| {
            if let FxEdit::Sharing(x) = e {
                Some(x)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(sharing.last().unwrap().synth_group, Some(4));
    s.shift_click("delay.on");
    s.shift_click("comp.on");
    s.shift_click("reverb.on");
    let last = s
        .fx_edits()
        .into_iter()
        .filter_map(|e| {
            if let FxEdit::Sharing(x) = e {
                Some(x)
            } else {
                None
            }
        })
        .next_back()
        .unwrap();
    assert_eq!(last.delay_global, Some(4));
    assert_eq!(last.comp_global, Some(4));
    assert_eq!(last.reverb_global, Some(4));
    assert!(!s.chain().delay_on, "shift+On toggles GLOBAL, not the unit");
}

#[test]
fn hold_on_a_unit_on_button_toggles_global() {
    let mut s = Session::new();
    s.click_for("comp.on", 0.7, Modifiers::NONE);
    assert!(s.state.fx_ui.global_comp.is_some());
    assert!(!s.chain().comp_on);
}

#[test]
fn layer_effects_on_gates_the_units_and_sends_neutral_values() {
    let mut s = Session::new();
    s.click("comp.on");
    s.click("delay.on");
    s.fx_edits();
    s.click("fx.layer_effects_on");
    assert!(!s.chain().all_on);
    let e = s.fx_edits();
    assert!(e.contains(&FxEdit::Flag {
        slot: 0,
        param: FxFlag::CompEnabled,
        on: false
    }));
    assert!(e.contains(&FxEdit::Flag {
        slot: 0,
        param: FxFlag::DelayEnabled,
        on: false
    }));
    assert!(e.contains(&FxEdit::ReverbModel {
        slot: 0,
        model: nord_core::fx::ReverbModel::Off
    }));
}

#[test]
fn tap_tempo_sets_the_delay_time() {
    let mut s = Session::new();
    s.idle = 0.4;
    for _ in 0..3 {
        s.click("delay.tap_set");
    }
    let t = s.chain().tempo_ms;
    assert!((400.0..=520.0).contains(&t), "tapped {t} ms");
    assert!(s.fx_edits().iter().any(|e| matches!(
        e,
        FxEdit::Float {
            param: FxFloat::DelayTime,
            ..
        }
    )));
}

#[test]
fn shift_turn_of_tempo_enters_master_clock_sync() {
    let mut s = Session::new();
    let p = s.center("delay.tempo");
    let m = Modifiers::SHIFT;
    s.frame(vec![Event::PointerMoved(p)], m);
    s.frame(vec![Session::button(p, true, m)], m);
    for i in 1..=6 {
        s.frame(
            vec![Event::PointerMoved(p - egui::vec2(0.0, 6.0 * i as f32))],
            m,
        );
    }
    s.frame(
        vec![Session::button(p - egui::vec2(0.0, 36.0), false, m)],
        m,
    );
    assert!(s.chain().mst > 0, "mst {}", s.chain().mst);
    assert!(s
        .fx_edits()
        .iter()
        .any(|e| matches!(e, FxEdit::DelaySync { beats: Some(_), .. })));
}

#[test]
fn amp_filter_and_to_rotary_are_exclusive_cabinet_choices() {
    let mut s = Session::new();
    s.click("fx.focus.piano");
    s.click("amp.filter");
    assert_eq!(s.chain().cabinet, AmpCabinet::LpFilter);
    s.click("amp.to_rotary");
    assert_eq!(s.chain().cabinet, AmpCabinet::ToRotary);
    s.click("amp.to_rotary");
    assert_eq!(s.chain().cabinet, AmpCabinet::Off);
}

#[test]
fn to_rotary_is_dimmed_and_inert_on_the_organ() {
    let mut s = Session::new();
    s.click("amp.to_rotary");
    assert_eq!(s.chain().cabinet, AmpCabinet::Off);
    assert!(s.fx_edits().is_empty());
}

#[test]
fn reverb_chorale_only_on_right_hand_types() {
    let mut s = Session::new();
    // default type is STAGE (index 3): VARIATION/CHORALE selects Chorale
    s.shift_click("reverb.type");
    assert!(s.fx_edits().contains(&FxEdit::ReverbColor {
        slot: 0,
        color: ReverbColor::Chorale
    }));
    s.state.fx_ui.chains[0].rev_sel = 1; // Booth: VARIATION instead of CHORALE
    s.click("reverb.on");
    s.click("reverb.on");
    let e = s.fx_edits();
    assert!(e.contains(&FxEdit::ReverbColor {
        slot: 0,
        color: ReverbColor::Normal
    }));
    assert!(
        e.contains(&FxEdit::Flag {
            slot: 0,
            param: FxFlag::ReverbVariation,
            on: true
        }),
        "the Booth VARIATION reaches the engine"
    );
}

#[test]
fn reverb_variation_toggles_on_the_left_hand_types_and_is_not_dimmed() {
    let mut s = Session::new();
    s.state.fx_ui.chains[0].rev_sel = 0; // Spring
    s.shift_click("reverb.type");
    assert!(s.chain().rev_alt);
    let e = s.fx_edits();
    assert!(e.contains(&FxEdit::Flag {
        slot: 0,
        param: FxFlag::ReverbVariation,
        on: true
    }));
    assert!(!e.contains(&FxEdit::ReverbColor {
        slot: 0,
        color: ReverbColor::Chorale
    }));
    s.shift_click("reverb.type");
    assert!(!s.chain().rev_alt);
    assert!(s.fx_edits().contains(&FxEdit::Flag {
        slot: 0,
        param: FxFlag::ReverbVariation,
        on: false
    }));
}

#[test]
fn program_load_changes_are_adopted_into_the_focused_chain() {
    let mut s = Session::new();
    s.state.mod2_type = nord_core::fx::Mod2Type::Vibe;
    s.state.delay_on = true;
    s.frame(Vec::new(), Modifiers::NONE);
    assert!(s.chain().mod2_on && s.chain().delay_on);
    assert_eq!(s.state.mod2_type, nord_core::fx::Mod2Type::Vibe);
    assert!(
        s.fx_edits().is_empty(),
        "adoption must not re-send to the engine"
    );
}

#[test]
fn unit_enum_covers_every_unit_with_edits() {
    let c = crate::panel::ui_state::fx::ChainUi::default();
    for u in [
        Unit::Mod1,
        Unit::Mod2,
        Unit::Amp,
        Unit::Comp,
        Unit::Delay,
        Unit::Reverb,
    ] {
        assert!(!c.edits(u, 0).is_empty());
    }
}

#[test]
fn mod_amount_knob_sends_only_the_amount_and_keeps_a_loaded_mod2() {
    use nord_core::engine::Stage4Engine;
    use nord_core::fx::Mod2Type;
    for (id, param) in [
        ("mod2.amount", FxFloat::Mod2Amount),
        ("mod1.amount", FxFloat::Mod1Amount),
    ] {
        let mut s = Session::new();
        // A loaded program: firmware CHOR with its variation on and the firmware words set.
        // The panel's `ChainUi` / `StateEcho` do not mirror `mod2_variation` (TECH_DEBT), so the
        // chain view still says "variation off"; a full Mod 2 edit set would write that over the
        // loaded flag and clear the words.
        let mut e = Stage4Engine::new(48_000.0);
        e.fx_chains[0].mod2_type = Mod2Type::Chorus;
        e.fx_chains[0].mod2_variation = true;
        e.fx_chains[0].mod2_type_raw = Some(4);
        e.fx_chains[0].mod2_rate_raw = Some(77);
        s.state.fx_ui.chains[0].mod2_on = true;
        s.drag_up(id, 20.0);
        let edits = s.fx_edits();
        assert!(
            edits
                .iter()
                .any(|x| matches!(x, FxEdit::Float { param: p, .. } if *p == param)),
            "{id}: no amount edit in {edits:?}"
        );
        assert!(
            edits
                .iter()
                .all(|x| matches!(x, FxEdit::Float { param: p, .. } if *p == param)),
            "{id}: only the amount may be sent, got {edits:?}"
        );
        for x in &edits {
            x.apply(&mut e);
        }
        let c = &e.fx_chains[0];
        assert_eq!(c.mod2_type, Mod2Type::Chorus, "{id}");
        assert!(c.mod2_variation, "{id}: loaded CHOR variation survives");
        assert_eq!(
            (c.mod2_type_raw, c.mod2_rate_raw),
            (Some(4), Some(77)),
            "{id}: firmware words survive"
        );
    }
}

#[test]
fn every_mod_control_sends_only_its_own_edits() {
    // Each touch must leave the other parameters of the unit alone: the full set would resend a
    // stale view (a loaded variation) and, for Mod 2, clear the firmware raw words.
    type Pick = fn(&FxEdit) -> bool;
    let is_type: Pick = |e| matches!(e, FxEdit::Mod1Type { .. } | FxEdit::Mod2Type { .. });
    let is_var: Pick = |e| {
        matches!(
            e,
            FxEdit::Flag {
                param: FxFlag::Mod1Variation | FxFlag::Mod2Variation,
                ..
            }
        )
    };
    let is_rate = |e: &FxEdit, p: FxFloat| matches!(e, FxEdit::Float { param, .. } if *param == p);
    for unit in ["mod1", "mod2"] {
        let rate_p = if unit == "mod1" {
            FxFloat::Mod1Rate
        } else {
            FxFloat::Mod2Rate
        };
        let amt_p = if unit == "mod1" {
            FxFloat::Mod1Amount
        } else {
            FxFloat::Mod2Amount
        };
        let (on, sel, rate, amt) = (
            format!("{unit}.on"),
            format!("{unit}.selector"),
            format!("{unit}.rate"),
            format!("{unit}.amount"),
        );
        // ON: type edit only.
        let mut s = Session::new();
        s.click(&on);
        let e = s.fx_edits();
        assert!(e.iter().any(is_type), "{unit} ON sends the type");
        assert!(
            e.iter().all(|x| is_type(x)
                || matches!(
                    x,
                    FxEdit::Mod1Sync { .. }
                        | FxEdit::Flag {
                            param: FxFlag::Mod1Ped,
                            ..
                        }
                )),
            "{unit} ON: {e:?}"
        );
        // Type selector: same.
        let mut s = Session::new();
        s.click(&sel);
        let e = s.fx_edits();
        assert!(e.iter().any(is_type));
        assert!(
            e.iter()
                .all(|x| !is_var(x) && !matches!(x, FxEdit::Float { .. })),
            "{unit} type: {e:?}"
        );
        // Shift+selector: variation only (Mod 1 type 0 has no PED).
        let mut s = Session::new();
        s.shift_click(&sel);
        let e = s.fx_edits();
        assert!(e.iter().any(is_var), "{unit} shift sends the variation");
        assert!(e.iter().all(is_var), "{unit} shift: {e:?}");
        // RATE: the rate only.
        let mut s = Session::new();
        s.drag_up(&rate, 20.0);
        let e = s.fx_edits();
        assert!(e.iter().any(|x| is_rate(x, rate_p)));
        assert!(e.iter().all(|x| is_rate(x, rate_p)), "{unit} rate: {e:?}");
        // AMOUNT: the amount only.
        let mut s = Session::new();
        s.drag_up(&amt, 20.0);
        let e = s.fx_edits();
        assert!(e.iter().all(|x| is_rate(x, amt_p)), "{unit} amount: {e:?}");
    }
}

#[test]
fn mod1_ped_knob_and_ped_toggle_send_only_their_edit() {
    let mut s = Session::new();
    s.state.fx_ui.chains[0].mod1_on = true;
    s.state.fx_ui.chains[0].mod1_sel = 4; // WAH has PED
    s.shift_click("mod1.selector");
    let e = s.fx_edits();
    assert!(
        e.iter().all(|x| matches!(
            x,
            FxEdit::Flag {
                param: FxFlag::Mod1Ped,
                ..
            }
        )),
        "PED toggle: {e:?}"
    );
    assert!(!e.is_empty());
    s.drag_up("mod1.rate", 20.0);
    let e = s.fx_edits();
    assert!(
        e.iter().all(|x| matches!(
            x,
            FxEdit::Float {
                param: FxFloat::Mod1PedPos,
                ..
            }
        )),
        "PED position: {e:?}"
    );
    assert!(!e.is_empty());
}
