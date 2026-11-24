//! Text size option for low-vision users (issue #592, bean `ns4-2v7q`).
//!
//! One setting, 100 to 200 percent in steps of 25, shared by the standalone app and the DAW plugin
//! editor. It scales everything egui draws as text: the top bar, menus, combo boxes, tooltips and
//! the Studio Settings dialog (egui style fonts and spacing, plus every explicit size through
//! [`px`]), and magnifies the baked plate so its printed legends grow the same factor
//! (`BakedZoom::base`, the "fit" view of the plate becomes `factor` times the fitted plate and the
//! user pans or jumps with keys `1`-`5`). The plate is a picture, so the only way to make its
//! print bigger in a fixed window is to show less of it; the planned double-stacked layout
//! (`docs/specs/visual-style-and-panel.md`, "Double-stacked layout") shows more of it at the same
//! size.
//!
//! egui's own zoom factor is not used: the plugin host (`egui_baseview`) lays out with the native
//! pixels-per-point, so a zoom factor only mis-sizes the surface there.
//!
//! The choice is stored as `text_scale_percent` in `<config dir>/ns4/config.json`
//! (`nord_core::user_config`). Keys: `Ctrl +` / `Ctrl -` / `Ctrl 0`; bar pills `A-` `NNN%` `A+`.
//!
//! ASSUMED: 25 % steps and the 200 % ceiling. They give the baked plate an exact 1.25 / 1.5 / 1.75 / 2
//! magnification and keep the top bar (about 800 points wide at 200 %, [`bar_content_width`]) inside
//! a window a 1366 point screen can hold; no reference exists for what low-vision users need beyond
//! the issue text. Confirm with the reporter.

use crate::panel::baked_zoom::BakedZoom;
use egui::{Context, Id, Key, Ui};
use std::cell::Cell;
use std::path::Path;

/// Smallest and largest text size, in percent.
pub const MIN_PERCENT: u32 = 100;
pub const MAX_PERCENT: u32 = 200;
/// Distance between two choices, in percent.
pub const STEP_PERCENT: u32 = 25;
/// Height of the top bar at 100 %, in points.
pub const BAR_HEIGHT_100: f32 = 22.0;

/// The chosen text size, snapped to [`STEP_PERCENT`] inside `MIN_PERCENT..=MAX_PERCENT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextScale(u32);

impl Default for TextScale {
    fn default() -> Self {
        Self(MIN_PERCENT)
    }
}

impl TextScale {
    /// The choice nearest to `percent`.
    pub fn new(percent: u32) -> Self {
        let p = percent.clamp(MIN_PERCENT, MAX_PERCENT);
        let steps = (p - MIN_PERCENT + STEP_PERCENT / 2) / STEP_PERCENT;
        Self((MIN_PERCENT + steps * STEP_PERCENT).min(MAX_PERCENT))
    }

    pub fn percent(self) -> u32 {
        self.0
    }

    pub fn factor(self) -> f32 {
        self.0 as f32 / 100.0
    }

    pub fn larger(self) -> Self {
        Self::new(self.0 + STEP_PERCENT)
    }

    pub fn smaller(self) -> Self {
        Self::new(self.0.saturating_sub(STEP_PERCENT))
    }
}

/// Loads the saved text size from `config` (`None`, a missing file, a damaged file: 100 %).
pub fn load(config: Option<&Path>) -> TextScale {
    let saved = config.and_then(|p| nord_core::user_config::load(p).ok());
    saved
        .and_then(|c| c.text_scale_percent)
        .map_or_else(TextScale::default, TextScale::new)
}

/// Saves `scale` in `config`, keeping the other saved settings.
pub fn save(config: Option<&Path>, scale: TextScale) -> Result<(), String> {
    let path = config.ok_or("no config directory on this platform")?;
    let mut cfg = nord_core::user_config::load(path).unwrap_or_default();
    cfg.text_scale_percent = Some(scale.percent());
    nord_core::user_config::save(path, &cfg)
}

thread_local! {
    /// Factor in force on this thread, read by [`px`]. egui draws a frame on one thread (the
    /// standalone UI thread, the editor window thread of a plugin instance) and [`apply`] sets it
    /// at the start of every frame, so two plugin instances with different sizes never mix, and
    /// parallel tests do not see each other.
    static FACTOR: Cell<f32> = const { Cell::new(1.0) };
}

/// The text factor in force on this thread (1.0 to 2.0).
pub fn factor() -> f32 {
    FACTOR.with(Cell::get)
}

/// `base` points at 100 %, scaled by the text size in force. Use for every explicit font size,
/// bar height and fixed control size of the egui chrome.
pub fn px(base: f32) -> f32 {
    base * factor()
}

/// Height of the top bar in points at the size in force.
pub fn bar_height() -> f32 {
    px(BAR_HEIGHT_100)
}

/// Width the bar buttons need (zoom pills, text size pills, `OPEN .NS4B`, `DEBUG`, `? HELP`) at the
/// size in force, in points: `268 + 265 x factor`.
/// EMPIRICAL: right edge of the last bar label measured by `bar_text_stays_whole_and_apart_at_every_size`
/// (nord-plugin `text_scale_tests.rs`): 530 / 663 / 797 pt at 100 / 150 / 200 %, fit within 1 pt
/// (`bar_width_formula_covers_the_measured_bar`).
pub fn bar_content_width() -> f32 {
    268.0 + 265.0 * factor()
}

/// Smallest plugin editor width: 640 points, or the bar plus a margin when that is wider.
pub fn editor_min_width(base: f32) -> f32 {
    base.max(bar_content_width() + 40.0)
}

/// Smallest standalone window width at the size in force: 960 points at 100 %, more when the bar
/// and the two window buttons need it, but never more than the monitor (`monitor_w`, points) so
/// the window can always shrink onto the screen and the window buttons stay reachable.
pub fn min_window_width(monitor_w: Option<f32>) -> f32 {
    let need = bar_content_width() + px(2.0 * 22.0 + 15.0) + 60.0;
    let w = 960.0_f32.max(need);
    monitor_w.map_or(w, |m| w.min(m.max(320.0)))
}

/// Smallest standalone window height at the size in force: the bar plus the 228 points of plate
/// that were the minimum at 100 % (250 points in all), never more than the monitor.
pub fn min_window_height(monitor_h: Option<f32>) -> f32 {
    let h = bar_height() + 228.0;
    monitor_h.map_or(h, |m| h.min(m.max(200.0)))
}

/// Scale the fonts and spacing of `style` from egui's defaults by `f` (idempotent: always derived
/// from the defaults, never from the current values).
pub fn scale_style(style: &mut egui::Style, f: f32) {
    let base = egui::Style::default();
    for (id, font) in &base.text_styles {
        let mut scaled = font.clone();
        scaled.size *= f;
        style.text_styles.insert(id.clone(), scaled);
    }
    let (b, s) = (&base.spacing, &mut style.spacing);
    s.item_spacing = b.item_spacing * f;
    s.button_padding = b.button_padding * f;
    s.interact_size = b.interact_size * f;
    s.icon_width = b.icon_width * f;
    s.icon_width_inner = b.icon_width_inner * f;
    s.icon_spacing = b.icon_spacing * f;
    s.indent = b.indent * f;
    s.combo_height = b.combo_height * f;
    s.slider_width = b.slider_width * f;
    s.text_edit_width = b.text_edit_width * f;
}

fn applied_id() -> Id {
    Id::new("ns4_text_scale_applied")
}

/// Make `ctx` and `zoom` show `scale`: the process-wide factor, the egui style and the plate
/// magnification. Call once per frame before drawing; it only does work when the size changed.
/// Returns whether anything changed.
pub fn apply(ctx: &Context, scale: TextScale, zoom: &mut BakedZoom) -> bool {
    let f = scale.factor();
    FACTOR.with(|c| c.set(f));
    let mut changed = false;
    if ctx.data(|d| d.get_temp::<u32>(applied_id())) != Some(scale.percent()) {
        // egui's built-in Ctrl +/- would change its zoom factor on top of ours.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        ctx.style_mut(|s| scale_style(s, f));
        ctx.data_mut(|d| d.insert_temp(applied_id(), scale.percent()));
        changed = true;
    }
    if (zoom.base - f).abs() > 1e-4 {
        zoom.set_base(f);
        changed = true;
    }
    changed
}

/// Keys of this frame: `Ctrl +` (or `Ctrl =`) larger, `Ctrl -` smaller, `Ctrl 0` back to 100 %.
pub fn handle_keys(ctx: &Context, scale: &mut TextScale) -> bool {
    // Ctrl + / - inside a text field belongs to the field.
    if ctx.wants_keyboard_input() {
        return false;
    }
    let next = ctx.input(|i| {
        if !i.modifiers.command {
            return None;
        }
        if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
            Some(scale.larger())
        } else if i.key_pressed(Key::Minus) {
            Some(scale.smaller())
        } else if i.key_pressed(Key::Num0) {
            Some(TextScale::default())
        } else {
            None
        }
    });
    match next {
        Some(n) if n != *scale => {
            *scale = n;
            true
        }
        _ => false,
    }
}

/// Bar pills `A-`, the percentage and `A+`; returns whether the size changed.
pub fn bar_controls(ui: &mut Ui, scale: &mut TextScale) -> bool {
    use crate::panel::baked_zoom::pill;
    let before = *scale;
    if pill(ui, "A-", false, "Smaller text (Ctrl -)").clicked() {
        *scale = scale.smaller();
    }
    let label = format!("{}%", scale.percent());
    if pill(ui, &label, false, "Text size: click for 100 % (Ctrl 0)").clicked() {
        *scale = TextScale::default();
    }
    if pill(ui, "A+", false, "Larger text (Ctrl +)").clicked() {
        *scale = scale.larger();
    }
    *scale != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_snap_to_steps_inside_the_range() {
        assert_eq!(TextScale::new(0).percent(), 100);
        assert_eq!(TextScale::new(112).percent(), 100);
        assert_eq!(TextScale::new(113).percent(), 125);
        assert_eq!(TextScale::new(150).percent(), 150);
        assert_eq!(TextScale::new(199).percent(), 200);
        assert_eq!(TextScale::new(9999).percent(), 200);
        assert_eq!(TextScale::default().factor(), 1.0);
        assert_eq!(TextScale::new(175).factor(), 1.75);
    }

    #[test]
    fn larger_and_smaller_stop_at_the_ends() {
        let mut s = TextScale::default();
        assert_eq!(s.smaller(), s);
        for want in [125, 150, 175, 200, 200] {
            s = s.larger();
            assert_eq!(s.percent(), want);
        }
        assert_eq!(s.smaller().percent(), 175);
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ns4-ts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("ns4").join("config.json")
    }

    #[test]
    fn the_size_persists_and_keeps_the_other_settings() {
        let path = tmp("persist");
        assert_eq!(load(Some(&path)), TextScale::default(), "no file: 100 %");
        let cfg = nord_core::user_config::UserConfig {
            os_update_path: Some("C:/dl/Update.exe".into()),
            ..Default::default()
        };
        nord_core::user_config::save(&path, &cfg).unwrap();
        save(Some(&path), TextScale::new(150)).unwrap();
        assert_eq!(load(Some(&path)).percent(), 150);
        let kept = nord_core::user_config::load(&path).unwrap();
        assert_eq!(kept.os_update_path.as_deref(), Some("C:/dl/Update.exe"));
        assert!(save(None, TextScale::default()).is_err());
        assert_eq!(load(None), TextScale::default());
    }

    #[test]
    fn a_damaged_or_out_of_range_file_falls_back_to_a_valid_size() {
        let path = tmp("damaged");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(Some(&path)), TextScale::default());
        std::fs::write(&path, r#"{ "text_scale_percent": 900 }"#).unwrap();
        assert_eq!(load(Some(&path)).percent(), 200);
        std::fs::write(&path, r#"{ "text_scale_percent": 3 }"#).unwrap();
        assert_eq!(load(Some(&path)).percent(), 100);
    }

    #[test]
    fn the_minimum_window_never_exceeds_the_monitor_and_holds_the_bar() {
        for pct in [100, 150, 200] {
            apply(
                &Context::default(),
                TextScale::new(pct),
                &mut BakedZoom::default(),
            );
            // A 1366 point screen: the window can always shrink onto it, at every size.
            assert!(min_window_width(Some(1366.0)) <= 1366.0, "{pct} %");
            assert!(min_window_height(Some(768.0)) <= 768.0, "{pct} %");
            // Without a monitor the bar (and the two window buttons) still fit.
            let need = bar_content_width() + px(2.0 * 22.0 + 15.0);
            assert!(min_window_width(None) >= need.max(960.0), "{pct} %");
        }
        apply(
            &Context::default(),
            TextScale::new(200),
            &mut BakedZoom::default(),
        );
        assert!(
            min_window_width(Some(1024.0)) <= 1024.0,
            "a small screen wins"
        );
        assert!(min_window_width(Some(10.0)) >= 320.0, "never degenerate");
        apply(
            &Context::default(),
            TextScale::default(),
            &mut BakedZoom::default(),
        );
        assert_eq!(min_window_width(None), 960.0, "100 %: the old minimum");
        assert_eq!(min_window_height(None), 250.0);
    }

    #[test]
    fn keys_do_nothing_while_a_text_field_has_the_keyboard() {
        let ctx = Context::default();
        let ctrl_plus = || egui::RawInput {
            modifiers: egui::Modifiers::COMMAND,
            events: vec![egui::Event::Key {
                key: Key::Equals,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        };
        let mut text = String::new();
        let mut scale = TextScale::default();
        // Focus a text field, then press Ctrl +.
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = ui.text_edit_singleline(&mut text);
                r.request_focus();
            });
        });
        let mut changed = true;
        let _ = ctx.run(ctrl_plus(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.text_edit_singleline(&mut text);
            });
            changed = handle_keys(ctx, &mut scale);
        });
        assert!(
            !changed && scale == TextScale::default(),
            "typing owns Ctrl +"
        );
        // Without a focused field the same key works.
        let ctx = Context::default();
        let mut changed = false;
        let _ = ctx.run(ctrl_plus(), |ctx| changed = handle_keys(ctx, &mut scale));
        assert!(changed && scale.percent() == 125);
    }

    #[test]
    fn style_scaling_is_derived_from_the_defaults_and_so_does_not_compound() {
        let base = egui::Style::default();
        let mut s = base.clone();
        for _ in 0..3 {
            scale_style(&mut s, 1.5);
        }
        let body = egui::TextStyle::Body;
        assert_eq!(
            s.text_styles[&body].size,
            base.text_styles[&body].size * 1.5
        );
        assert_eq!(s.spacing.interact_size, base.spacing.interact_size * 1.5);
        scale_style(&mut s, 1.0);
        assert_eq!(s.text_styles, base.text_styles);
        assert_eq!(s.spacing.item_spacing, base.spacing.item_spacing);
    }
}
