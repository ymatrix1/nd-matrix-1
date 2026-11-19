//! Section plate with its light header rim and raised header tab (shape: `nord_panel::layout::plate`).

use super::basic::label;
use crate::style::{HEADER_DARK, HEADER_EDGE, HEADER_INK, MODULE_SLATE, MODULE_SLATE_DARK};
use egui::{pos2, Align2, Painter, Rect, Stroke, StrokeKind};

/// Title font (px) in a tab / in the bare rim of a tab-less plate.
const TITLE_PX: f32 = 12.0;
const RIM_TITLE_PX: f32 = 9.0;

/// Plate of one section: slate body from `top` (screen y of the plate's top edge) to the bottom of
/// `section`, a light `rim` strip along `top`, and an optional header `tab` above it that carries
/// `title`. A plate without a tab puts the (smaller) title in the rim.
pub fn plate_frame(p: &Painter, section: Rect, top: f32, rim: f32, tab: Option<Rect>, title: &str) {
    let body = Rect::from_min_max(pos2(section.left(), top), section.max);
    p.rect_filled(body, 5.0, MODULE_SLATE);
    let strip = Rect::from_min_size(body.min, egui::vec2(body.width(), rim));
    p.rect_filled(strip, 5.0, HEADER_DARK);
    // Square off the rim's lower corners.
    p.rect_filled(
        Rect::from_min_size(
            pos2(strip.left(), strip.bottom() - 5.0),
            egui::vec2(strip.width(), 5.0),
        ),
        0.0,
        HEADER_DARK,
    );
    // Outline before the tab: the raised tab covers the outline's top edge (a line through the
    // title and the ON button otherwise).
    p.rect_stroke(
        body,
        5.0,
        Stroke::new(1.0_f32, MODULE_SLATE_DARK),
        StrokeKind::Inside,
    );
    let ink = HEADER_INK;
    match tab {
        Some(t) => {
            p.rect_filled(t, 5.0, HEADER_DARK);
            // The tab joins the rim without a seam on its lower edge.
            p.rect_filled(
                Rect::from_min_size(pos2(t.left(), t.bottom() - 6.0), egui::vec2(t.width(), 6.0)),
                0.0,
                HEADER_DARK,
            );
            label(
                p,
                pos2(t.left() + 8.0, t.center().y),
                title,
                TITLE_PX,
                ink,
                Align2::LEFT_CENTER,
            );
            p.rect_stroke(
                t,
                5.0,
                Stroke::new(1.0_f32, HEADER_EDGE),
                StrokeKind::Inside,
            );
        }
        None => {
            label(
                p,
                pos2(strip.left() + 8.0, strip.center().y),
                title,
                RIM_TITLE_PX,
                ink,
                Align2::LEFT_CENTER,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn paints_with_and_without_a_tab() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            let sec = Rect::from_min_size(pos2(0.0, 60.0), vec2(300.0, 300.0));
            let tab = Rect::from_min_size(sec.min, vec2(120.0, 24.0));
            let n0 = ctx.input(|i| i.time);
            plate_frame(&p, sec, 74.0, 10.0, Some(tab), "ORGAN");
            plate_frame(&p, sec, 74.0, 10.0, None, "SYNTH");
            assert!(n0 >= 0.0);
        });
    }

    /// Regression: the body outline used to be painted last, so its top edge ran through the
    /// header tab (a line struck through "ORGAN" and behind the ON button).
    #[test]
    fn tab_is_painted_over_the_body_outline() {
        let ctx = egui::Context::default();
        let sec = Rect::from_min_size(pos2(0.0, 60.0), vec2(300.0, 300.0));
        let tab = Rect::from_min_size(sec.min, vec2(120.0, 24.0));
        let out = ctx.run(Default::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            plate_frame(&p, sec, 74.0, 10.0, Some(tab), "ORGAN");
        });
        let body = Rect::from_min_max(pos2(0.0, 74.0), sec.max);
        let rects: Vec<(usize, Rect, bool)> = out
            .shapes
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match &s.shape {
                egui::Shape::Rect(r) => Some((i, r.rect, !r.stroke.is_empty())),
                _ => None,
            })
            .collect();
        let outline = rects.iter().position(|(_, r, st)| *st && *r == body);
        let tab_fill = rects.iter().position(|(_, r, st)| !*st && *r == tab);
        let (Some(o), Some(t)) = (outline, tab_fill) else {
            panic!("outline {outline:?} / tab fill {tab_fill:?} not painted");
        };
        assert!(
            o < t,
            "body outline ({o}) must be painted before the tab ({t})"
        );
    }

    /// Header text must stay readable on the graphite header (WCAG contrast >= 7).
    #[test]
    fn header_ink_contrast_on_graphite_is_at_least_7() {
        fn lum(c: egui::Color32) -> f32 {
            let f = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        }
        let (a, b) = (lum(HEADER_INK), lum(HEADER_DARK));
        let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        assert!(ratio >= 7.0, "contrast {ratio}");
    }
}
