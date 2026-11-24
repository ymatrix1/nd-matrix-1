//! Zoom and pan of the baked front plate (`baked_view.rs`).
//!
//! The baked plate is one fixed-aspect picture (about 4:1). Fitted to a normal window its text and
//! controls are small, so the view can zoom into one bay or any spot and pan around. Everything
//! else in `baked_view.rs` already derives its rectangles from the plate rectangle, so zoom is only
//! a different plate rectangle plus a clip to the viewport.
//!
//! * `zoom == 1` is the whole plate fitted to the viewport (the old behaviour).
//! * Keys: `1`-`5` jump to Organ / Piano / Program / Synth / Effects, `0` or `Esc` fit, `+` / `-`
//!   step. Ctrl + scroll or a pinch zooms around the pointer. Dragging empty plate pans.
//! * Bar pills (`bar_controls`): `-`, percentage, `+`, `FIT`, and one pill per section.
//!
//! ASSUMED: `MAX_ZOOM` 6 and `STEP` 1.25, chosen so the smallest bay (Piano) fills a 1760 px
//! window with room to spare; no hardware reference exists for a software zoom.

use egui::{pos2, vec2, Color32, CursorIcon, Key, Rect, RichText, Sense, Stroke, Ui, Vec2};
use nord_panel::Section;
use std::collections::BTreeMap;

use crate::baked::BakedRect;

/// Whole plate fitted to the viewport.
pub const MIN_ZOOM: f32 = 1.0;
/// Largest zoom (plate widths per fitted width).
pub const MAX_ZOOM: f32 = 6.0;
/// Factor of one `+` / `-` step.
pub const STEP: f32 = 1.25;
/// Normalized plate margin kept around a focused bay.
const MARGIN: f32 = 0.012;
/// Smallest plate width kept when the viewport is tiny (matches `baked_view`).
const MIN_PLATE_W: f32 = 320.0;

/// Sections reachable with keys `1`-`5` and the bar pills, in key order.
pub const SECTIONS: [(Section, &str, &str); 5] = [
    (Section::Organ, "bay-organ", "ORG"),
    (Section::Piano, "bay-piano", "PNO"),
    (Section::Program, "bay-program", "PRG"),
    (Section::Synth, "bay-synth", "SYN"),
    (Section::Effects, "bay-fx", "FX"),
];

/// A zoom change asked for by a key or a bar pill, applied once the viewport is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomRequest {
    Fit,
    In,
    Out,
    Section(Section),
}

/// Zoom state of the baked plate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakedZoom {
    /// Plate size relative to the fitted plate (`1` = whole plate).
    pub zoom: f32,
    /// Plate point (normalized `0..1`) at the viewport centre.
    pub center: Vec2,
    /// Bay the last jump went to, `None` after any other change.
    pub section: Option<Section>,
    /// Pending request from the bar; applied by `update`.
    pub request: Option<ZoomRequest>,
    /// Bay under the pointer on empty plate (not over a control), for the hover highlight.
    pub hover: Option<Section>,
    /// Zoom of the "fit" view: 1 shows the whole plate, the text size option (`text_scale.rs`)
    /// makes it 1.25 to 2 so the printed legends grow; `Fit` returns to it, panning stays free.
    pub base: f32,
}

impl Default for BakedZoom {
    fn default() -> Self {
        Self {
            zoom: MIN_ZOOM,
            center: vec2(0.5, 0.5),
            section: None,
            request: None,
            hover: None,
            base: MIN_ZOOM,
        }
    }
}

/// Plate size that fits `view` at zoom 1 for a plate of `aspect` (width / height).
pub fn fit_size(view: Vec2, aspect: f32) -> Vec2 {
    let w = view.x.min(view.y * aspect).max(MIN_PLATE_W);
    vec2(w, w / aspect)
}

fn bay_rect(bays: &BTreeMap<String, BakedRect>, name: &str) -> Option<Rect> {
    let b = bays.get(name)?;
    Some(Rect::from_min_size(pos2(b.x, b.y), vec2(b.w, b.h)))
}

impl BakedZoom {
    /// Plate rectangle for `view`: at zoom 1 top-aligned and centred like the unzoomed plate, when
    /// zoomed it never leaves a gap inside the viewport.
    pub fn plate_rect(&self, view: Rect, aspect: f32) -> Rect {
        let size = fit_size(view.size(), aspect) * self.zoom;
        let axis = |view_min: f32, view_len: f32, len: f32, centre: f32, top_aligned: bool| {
            if len <= view_len {
                if top_aligned {
                    view_min
                } else {
                    view_min + (view_len - len) * 0.5
                }
            } else {
                let min = view_min + view_len * 0.5 - centre * len;
                min.clamp(view_min + view_len - len, view_min)
            }
        };
        let min = pos2(
            axis(view.left(), view.width(), size.x, self.center.x, false),
            axis(view.top(), view.height(), size.y, self.center.y, true),
        );
        Rect::from_min_size(min, size)
    }

    /// Re-derive `center` from the clamped plate rectangle so it never drifts out of range.
    fn normalize(&mut self, view: Rect, aspect: f32) {
        self.zoom = self.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let plate = self.plate_rect(view, aspect);
        let c = |view_c: f32, min: f32, len: f32, fits: bool| {
            if fits {
                0.5
            } else {
                (view_c - min) / len
            }
        };
        self.center = vec2(
            c(
                view.center().x,
                plate.left(),
                plate.width(),
                plate.width() <= view.width(),
            ),
            c(
                view.center().y,
                plate.top(),
                plate.height(),
                plate.height() <= view.height(),
            ),
        );
    }

    /// Fit the whole plate.
    pub fn fit(&mut self) {
        *self = Self {
            zoom: self.base,
            base: self.base,
            ..Self::default()
        };
    }

    /// Set the zoom of the fit view; a view that was at the old fit view follows it.
    pub fn set_base(&mut self, base: f32) {
        let at_fit = self.zoom <= self.base + 1e-3;
        self.base = base.clamp(MIN_ZOOM, MAX_ZOOM);
        // A view smaller than the new fit view would show less print than the setting promises.
        if at_fit || self.zoom < self.base {
            self.request = Some(ZoomRequest::Fit);
        }
    }

    /// Zoom so the normalized `rect` fills `view` (with a small margin), centred on it.
    pub fn focus_rect(&mut self, rect: Rect, view: Rect, aspect: f32) {
        let r = rect.expand(MARGIN);
        let f = fit_size(view.size(), aspect);
        self.zoom = (view.width() / (f.x * r.width()))
            .min(view.height() / (f.y * r.height()))
            .clamp(MIN_ZOOM, MAX_ZOOM);
        self.center = r.center().to_vec2();
        self.section = None;
        self.normalize(view, aspect);
    }

    /// Multiply the zoom by `factor`, keeping the plate point under `anchor` where it is.
    pub fn zoom_about(&mut self, factor: f32, anchor: egui::Pos2, view: Rect, aspect: f32) {
        let old = self.plate_rect(view, aspect);
        let norm = vec2(
            (anchor.x - old.left()) / old.width(),
            (anchor.y - old.top()) / old.height(),
        );
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let size = fit_size(view.size(), aspect) * self.zoom;
        let min = pos2(anchor.x - norm.x * size.x, anchor.y - norm.y * size.y);
        self.center = vec2(
            (view.center().x - min.x) / size.x,
            (view.center().y - min.y) / size.y,
        );
        self.section = None;
        self.normalize(view, aspect);
    }

    /// Move the picture by `delta` screen px (a drag).
    pub fn pan_by(&mut self, delta: Vec2, view: Rect, aspect: f32) {
        let size = fit_size(view.size(), aspect) * self.zoom;
        self.center -= vec2(delta.x / size.x, delta.y / size.y);
        self.section = None;
        self.normalize(view, aspect);
    }

    /// Apply a request now that the viewport is known.
    pub fn apply(
        &mut self,
        req: ZoomRequest,
        view: Rect,
        aspect: f32,
        bays: &BTreeMap<String, BakedRect>,
    ) {
        match req {
            ZoomRequest::Fit => {
                self.fit();
                if self.base > MIN_ZOOM {
                    // A magnified fit view starts at the top left, where the reading starts.
                    self.center = Vec2::ZERO;
                    self.normalize(view, aspect);
                }
            }
            ZoomRequest::In => self.zoom_about(STEP, view.center(), view, aspect),
            ZoomRequest::Out => self.zoom_about(1.0 / STEP, view.center(), view, aspect),
            ZoomRequest::Section(s) => {
                let name = SECTIONS.iter().find(|x| x.0 == s).map(|x| x.1);
                if let Some(rect) = name.and_then(|n| bay_rect(bays, n)) {
                    self.focus_rect(rect, view, aspect);
                    self.section = Some(s);
                }
            }
        }
    }

    /// Percentage shown in the bar (`100` = whole plate fitted).
    pub fn percent(&self) -> u32 {
        (self.zoom * 100.0).round() as u32
    }

    /// Per-frame step: pending request, keys, Ctrl+scroll / pinch, and the pan drag. Call before the
    /// controls are registered so they win over the drag surface.
    pub fn update(
        &mut self,
        ui: &mut Ui,
        view: Rect,
        aspect: f32,
        bays: &BTreeMap<String, BakedRect>,
    ) {
        let req = self
            .request
            .take()
            .or_else(|| key_request(ui.ctx(), self.zoom / self.base));
        if let Some(req) = req {
            self.apply(req, view, aspect, bays);
        }
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = pointer.filter(|p| view.contains(*p));
        if let Some(p) = over {
            let factor = ui.input(|i| i.zoom_delta());
            if (factor - 1.0).abs() > 1e-3 {
                self.zoom_about(factor, p, view, aspect);
            }
        }
        let resp = ui.interact(
            view,
            ui.id().with("baked_zoom_pan"),
            Sense::click_and_drag(),
        );
        // `hovered` is false over a control (they are registered after this surface and win).
        let plate = self.plate_rect(view, aspect);
        self.hover = over
            .filter(|_| resp.hovered() || resp.dragged())
            .and_then(|p| section_at(p, plate, bays));
        if resp.dragged() && self.zoom > MIN_ZOOM + 1e-3 {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            self.pan_by(resp.drag_delta(), view, aspect);
        } else if resp.clicked() {
            if let Some(s) = self.hover {
                // A second click on the focused bay goes back to the whole panel.
                let req = if self.section == Some(s) {
                    ZoomRequest::Fit
                } else {
                    ZoomRequest::Section(s)
                };
                self.apply(req, view, aspect, bays);
            }
        } else if let Some(s) = self.hover {
            let tip = if self.section == Some(s) {
                format!("Click: whole panel (0 or Esc). Drag: pan. {s:?}")
            } else {
                format!("Click to zoom to {s:?} ({})", section_key(s))
            };
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            let _ = resp.on_hover_text(tip);
        } else if self.zoom > MIN_ZOOM + 1e-3 && resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        // A window resize or a new aspect can invalidate the stored centre.
        self.normalize(view, aspect);
    }
}

/// Bay of `SECTIONS` under `pos` for a plate drawn at `plate`.
pub fn section_at(
    pos: egui::Pos2,
    plate: Rect,
    bays: &BTreeMap<String, BakedRect>,
) -> Option<Section> {
    SECTIONS.iter().find_map(|(s, name, _)| {
        let r = bay_rect(bays, name)?;
        let px = Rect::from_min_size(
            pos2(
                plate.left() + r.left() * plate.width(),
                plate.top() + r.top() * plate.height(),
            ),
            vec2(r.width() * plate.width(), r.height() * plate.height()),
        );
        px.contains(pos).then_some(*s)
    })
}

/// Key that jumps to `s` (`1`..`5`), for tooltips.
fn section_key(s: Section) -> usize {
    SECTIONS.iter().position(|x| x.0 == s).map_or(0, |n| n + 1)
}

impl BakedZoom {
    /// Highlight the hovered bay (call right after the plate is painted).
    pub fn paint_hover(
        &self,
        painter: &egui::Painter,
        plate: Rect,
        bays: &BTreeMap<String, BakedRect>,
    ) {
        let Some(s) = self.hover else { return };
        let Some(r) = SECTIONS
            .iter()
            .find(|x| x.0 == s)
            .and_then(|x| bay_rect(bays, x.1))
        else {
            return;
        };
        let px = Rect::from_min_size(
            pos2(
                plate.left() + r.left() * plate.width(),
                plate.top() + r.top() * plate.height(),
            ),
            vec2(r.width() * plate.width(), r.height() * plate.height()),
        );
        painter.rect_filled(px, 6.0, Color32::from_white_alpha(22));
        painter.rect_stroke(
            px,
            6.0,
            Stroke::new(1.5_f32, Color32::from_white_alpha(150)),
            egui::StrokeKind::Inside,
        );
    }
}

fn digit(n: usize) -> Key {
    [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5][n]
}

/// Request asked for by this frame's keys: `1`-`5`, `0`, `Esc` (only while zoomed), `+`, `-`.
/// Ignored while a text field has the keyboard or a modifier is down (Ctrl+/- is egui's own zoom).
pub fn key_request(ctx: &egui::Context, zoom: f32) -> Option<ZoomRequest> {
    if ctx.wants_keyboard_input() {
        return None;
    }
    ctx.input(|i| {
        if i.modifiers.any() {
            return None;
        }
        if let Some(n) = (0..SECTIONS.len()).find(|n| i.key_pressed(digit(*n))) {
            return Some(ZoomRequest::Section(SECTIONS[n].0));
        }
        if i.key_pressed(Key::Num0) || (i.key_pressed(Key::Escape) && zoom > MIN_ZOOM + 1e-3) {
            return Some(ZoomRequest::Fit);
        }
        if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
            return Some(ZoomRequest::In);
        }
        if i.key_pressed(Key::Minus) {
            return Some(ZoomRequest::Out);
        }
        None
    })
}

pub(crate) fn pill(ui: &mut Ui, label: &str, active: bool, tip: &str) -> egui::Response {
    let fill = if active {
        Color32::from_rgb(184, 37, 42)
    } else {
        Color32::from_rgba_unmultiplied(20, 22, 26, 200)
    };
    ui.add(
        egui::Button::new(
            RichText::new(label)
                .size(crate::text_scale::px(9.0))
                .strong()
                .color(Color32::from_rgb(236, 240, 244)),
        )
        .fill(fill)
        .stroke(Stroke::new(
            0.8_f32,
            Color32::from_rgba_unmultiplied(255, 255, 255, 45),
        ))
        .corner_radius(4.0),
    )
    .on_hover_text(tip)
    .on_hover_cursor(CursorIcon::PointingHand)
}

/// Zoom pills for the top bar: `-`, percentage, `+`, `FIT`, then one pill per section. Sets
/// `zoom.request`; the plate applies it on the next frame.
pub fn bar_controls(ui: &mut Ui, zoom: &mut BakedZoom) {
    if pill(ui, "-", false, "Zoom out (-)").clicked() {
        zoom.request = Some(ZoomRequest::Out);
    }
    ui.label(
        RichText::new(format!("{}%", zoom.percent()))
            .size(crate::text_scale::px(9.0))
            .strong()
            .color(Color32::from_rgb(236, 240, 244)),
    );
    if pill(
        ui,
        "+",
        false,
        "Zoom in (+). Ctrl+scroll zooms at the pointer; drag the plate to pan",
    )
    .clicked()
    {
        zoom.request = Some(ZoomRequest::In);
    }
    if pill(
        ui,
        "FIT",
        zoom.zoom <= zoom.base + 1e-3,
        "Whole panel (0 or Esc)",
    )
    .clicked()
    {
        zoom.request = Some(ZoomRequest::Fit);
    }
    for (n, (section, _, label)) in SECTIONS.iter().enumerate() {
        let tip = format!("Zoom to {section:?} ({})", n + 1);
        if pill(ui, label, zoom.section == Some(*section), &tip).clicked() {
            zoom.request = Some(ZoomRequest::Section(*section));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASPECT: f32 = 2632.0 / 650.0;

    fn view() -> Rect {
        Rect::from_min_size(pos2(0.0, 22.0), vec2(1760.0, 414.0))
    }

    fn bays() -> BTreeMap<String, BakedRect> {
        let mut m = BTreeMap::new();
        let r = |x, y, w, h| BakedRect {
            x,
            y,
            w,
            h,
            px: [0; 4],
        };
        m.insert("bay-organ".into(), r(0.095, 0.048, 0.187, 0.572));
        m.insert("bay-piano".into(), r(0.284, 0.048, 0.093, 0.572));
        m
    }

    #[test]
    fn zoom_one_is_the_fitted_plate_top_aligned() {
        let z = BakedZoom::default();
        let p = z.plate_rect(view(), ASPECT);
        assert!(
            (p.width() - 414.0 * ASPECT).abs() < 0.5,
            "limited by the height"
        );
        assert_eq!(p.top(), view().top());
        assert!((p.center().x - view().center().x).abs() < 0.5);
    }

    #[test]
    fn the_zoomed_plate_always_covers_the_viewport() {
        let v = view();
        let mut z = BakedZoom::default();
        z.apply(ZoomRequest::Section(Section::Piano), v, ASPECT, &bays());
        let p = z.plate_rect(v, ASPECT);
        assert!(p.left() <= v.left() + 0.01 && p.right() >= v.right() - 0.01);
        assert!(p.top() <= v.top() + 0.01 && p.bottom() >= v.bottom() - 0.01);
        // Panning far past the edge stays clamped.
        z.pan_by(vec2(1.0e6, 1.0e6), v, ASPECT);
        let p = z.plate_rect(v, ASPECT);
        assert!(p.left() <= v.left() + 0.01 && p.top() <= v.top() + 0.01);
        z.pan_by(vec2(-1.0e6, -1.0e6), v, ASPECT);
        let p = z.plate_rect(v, ASPECT);
        assert!(p.right() >= v.right() - 0.01 && p.bottom() >= v.bottom() - 0.01);
    }

    #[test]
    fn a_section_jump_puts_the_bay_inside_the_viewport_and_bigger() {
        let v = view();
        let mut z = BakedZoom::default();
        let before = z.plate_rect(v, ASPECT).width();
        z.apply(ZoomRequest::Section(Section::Organ), v, ASPECT, &bays());
        let p = z.plate_rect(v, ASPECT);
        assert!(
            p.width() > before * 1.5,
            "organ bay is about a fifth of the plate"
        );
        let b = bays()["bay-organ"].clone();
        let bay = Rect::from_min_size(
            pos2(p.left() + b.x * p.width(), p.top() + b.y * p.height()),
            vec2(b.w * p.width(), b.h * p.height()),
        );
        assert!(v.contains_rect(bay), "bay {bay:?} inside {v:?}");
        assert_eq!(z.section, Some(Section::Organ));
    }

    #[test]
    fn section_at_finds_the_bay_under_the_pointer() {
        let z = BakedZoom::default();
        let plate = z.plate_rect(view(), ASPECT);
        let b = bays();
        let inside = |name: &str| {
            let r = &b[name];
            pos2(
                plate.left() + (r.x + r.w * 0.5) * plate.width(),
                plate.top() + (r.y + r.h * 0.5) * plate.height(),
            )
        };
        assert_eq!(
            section_at(inside("bay-organ"), plate, &b),
            Some(Section::Organ)
        );
        assert_eq!(
            section_at(inside("bay-piano"), plate, &b),
            Some(Section::Piano)
        );
        assert_eq!(section_at(plate.left_top(), plate, &b), None);
    }

    #[test]
    fn zoom_is_clamped_and_fit_resets() {
        let v = view();
        let mut z = BakedZoom::default();
        for _ in 0..40 {
            z.apply(ZoomRequest::In, v, ASPECT, &bays());
        }
        assert!((z.zoom - MAX_ZOOM).abs() < 1e-4);
        for _ in 0..80 {
            z.apply(ZoomRequest::Out, v, ASPECT, &bays());
        }
        assert!((z.zoom - MIN_ZOOM).abs() < 1e-4);
        z.apply(ZoomRequest::In, v, ASPECT, &bays());
        z.apply(ZoomRequest::Fit, v, ASPECT, &bays());
        assert_eq!(z, BakedZoom::default());
        assert_eq!(z.percent(), 100);
    }

    #[test]
    fn zoom_about_keeps_the_plate_point_under_the_pointer() {
        let v = view();
        let mut z = BakedZoom::default();
        z.apply(ZoomRequest::In, v, ASPECT, &bays());
        let anchor = pos2(500.0, 200.0);
        let p0 = z.plate_rect(v, ASPECT);
        let n = (
            (anchor.x - p0.left()) / p0.width(),
            (anchor.y - p0.top()) / p0.height(),
        );
        z.zoom_about(1.25, anchor, v, ASPECT);
        let p1 = z.plate_rect(v, ASPECT);
        let n1 = (
            (anchor.x - p1.left()) / p1.width(),
            (anchor.y - p1.top()) / p1.height(),
        );
        assert!(
            (n.0 - n1.0).abs() < 1e-3 && (n.1 - n1.1).abs() < 1e-3,
            "{n:?} {n1:?}"
        );
    }

    fn press(key: Key, mods: egui::Modifiers) -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: mods,
            }],
            modifiers: mods,
            ..Default::default()
        }
    }

    fn request(key: Key, mods: egui::Modifiers, zoom: f32) -> Option<ZoomRequest> {
        let ctx = egui::Context::default();
        let mut out = None;
        let _ = ctx.run(press(key, mods), |ctx| out = key_request(ctx, zoom));
        out
    }

    #[test]
    fn keys_map_to_requests() {
        let none = egui::Modifiers::NONE;
        assert_eq!(
            request(Key::Num1, none, 1.0),
            Some(ZoomRequest::Section(Section::Organ))
        );
        assert_eq!(
            request(Key::Num5, none, 1.0),
            Some(ZoomRequest::Section(Section::Effects))
        );
        assert_eq!(request(Key::Num0, none, 2.0), Some(ZoomRequest::Fit));
        assert_eq!(request(Key::Escape, none, 2.0), Some(ZoomRequest::Fit));
        assert_eq!(
            request(Key::Escape, none, 1.0),
            None,
            "Esc is free at zoom 1"
        );
        assert_eq!(request(Key::Plus, none, 1.0), Some(ZoomRequest::In));
        assert_eq!(request(Key::Minus, none, 1.0), Some(ZoomRequest::Out));
        assert_eq!(request(Key::Num1, egui::Modifiers::CTRL, 1.0), None);
    }

    #[test]
    fn bar_controls_draw_and_start_without_a_request() {
        let ctx = egui::Context::default();
        let mut z = BakedZoom::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| bar_controls(ui, &mut z));
        });
        assert_eq!(z.request, None);
    }

    #[test]
    fn the_five_sections_are_bays_of_the_baked_layout() {
        let json = include_str!("../../assets/baked/panel_layout.json");
        let layout: crate::baked::BakedPanelLayout = serde_json::from_str(json).expect("layout");
        for (_, bay, _) in SECTIONS {
            assert!(layout.bays.contains_key(bay), "{bay}");
        }
    }
}
