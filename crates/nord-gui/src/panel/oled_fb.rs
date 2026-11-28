//! The OLED as a 128 x 64 frame buffer, shown as a smoothly scaled texture.
//!
//! The painters lay the display out on the real 128 x 64 grid (one egui point per display pixel).
//! Drawing those shapes straight to the screen at 1.3 or 2.7 screen pixels per display pixel makes
//! strokes alternate between thin and thick. Instead the shapes are rasterised once here into
//! coverage values (`0.0` dark to `1.0` lit), kept as a texture and scaled to the panel with
//! linear filtering, so every display pixel gets the same size and softness at any zoom.
//!
//! Supported shapes: filled and stroked rectangles, line segments, filled circles and polylines.
//! Text shapes are not supported (the firmware bitmap fonts are rectangles); callers only use this
//! path when the firmware fonts are installed.

use crate::panel::prelude::*;
use egui::epaint::ColorMode;
use egui::epaint::Shape;
use egui::{ColorImage, Id, Pos2, TextureHandle, TextureOptions, Vec2};
use std::hash::{Hash, Hasher};

/// Display width in pixels.
pub const W: usize = 128;
/// Display height in pixels.
pub const H: usize = 64;
/// From this many screen pixels per display pixel the texture is shown unsmoothed (sharp pixels).
const NEAREST_FROM_CELL: f32 = 4.0;
/// Sub-samples per axis for the diagonal lines and circles.
const SUB: usize = 4;

/// Coverage frame, row-major, one value in `0.0..=1.0` per display pixel.
pub type Frame = Vec<f32>;

/// Lit level of `c` between the OLED background (0) and `fg` (1).
fn level(c: Color32, fg: Color32) -> f32 {
    let bg = f32::from(OLED_BG.g());
    let own = f32::from(c.g()) - bg * f32::from(c.a()) / 255.0;
    (own / (f32::from(fg.g()) - bg)).clamp(0.0, 1.0)
}

/// Blends `level` into pixel `(x, y)` with weight `a`.
fn blend(f: &mut Frame, x: usize, y: usize, a: f32, level: f32) {
    let p = &mut f[y * W + x];
    *p = *p * (1.0 - a) + level * a;
}

/// Overlap of `[lo, hi)` with the pixel `[i, i + 1)`.
fn overlap(lo: f32, hi: f32, i: usize) -> f32 {
    (hi.min(i as f32 + 1.0) - lo.max(i as f32)).max(0.0)
}

/// Area of pixel `(x, y)` covered by `r`.
fn cover(r: Rect, x: usize, y: usize) -> f32 {
    overlap(r.left(), r.right(), x) * overlap(r.top(), r.bottom(), y)
}

/// Pixel index range touched by `[lo, hi)` on an axis of `n` pixels.
fn span(lo: f32, hi: f32, n: usize) -> std::ops::Range<usize> {
    let a = lo.floor().max(0.0) as usize;
    let b = (hi.ceil().max(0.0) as usize).min(n);
    a.min(n)..b
}

fn fill_rect(f: &mut Frame, r: Rect, lvl: f32) {
    for y in span(r.top(), r.bottom(), H) {
        for x in span(r.left(), r.right(), W) {
            blend(f, x, y, cover(r, x, y), lvl);
        }
    }
}

/// A ring: `outer` minus `inner`.
fn stroke_rect(f: &mut Frame, outer: Rect, inner: Rect, lvl: f32) {
    for y in span(outer.top(), outer.bottom(), H) {
        for x in span(outer.left(), outer.right(), W) {
            let a = cover(outer, x, y) - cover(inner, x, y).min(cover(outer, x, y));
            blend(f, x, y, a, lvl);
        }
    }
}

/// Distance from `p` to the segment `a`-`b`.
fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() == 0.0 {
        0.0
    } else {
        ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
    };
    (p - (a + ab * t)).length()
}

/// Coverage by supersampling a shape given as a point-in test.
fn fill_by(f: &mut Frame, bounds: Rect, lvl: f32, inside: impl Fn(Pos2) -> bool) {
    for y in span(bounds.top(), bounds.bottom(), H) {
        for x in span(bounds.left(), bounds.right(), W) {
            let mut n = 0;
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let p = pos2(
                        x as f32 + (sx as f32 + 0.5) / SUB as f32,
                        y as f32 + (sy as f32 + 0.5) / SUB as f32,
                    );
                    n += usize::from(inside(p));
                }
            }
            blend(f, x, y, n as f32 / (SUB * SUB) as f32, lvl);
        }
    }
}

/// One line segment with a stroke at least one display pixel wide. A horizontal or vertical line
/// sits on the pixel row or column its coordinate falls in (as the firmware's one-pixel lines do).
fn line(f: &mut Frame, a: Pos2, b: Pos2, width: f32, lvl: f32) {
    let w = width.max(1.0);
    if a.y == b.y {
        let y = a.y.floor();
        let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
        fill_rect(f, Rect::from_min_max(pos2(x0, y), pos2(x1, y + w)), lvl);
    } else if a.x == b.x {
        let x = a.x.floor();
        let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
        fill_rect(f, Rect::from_min_max(pos2(x, y0), pos2(x + w, y1)), lvl);
    } else {
        let bounds = Rect::from_two_pos(a, b).expand(w);
        fill_by(f, bounds, lvl, |p| seg_dist(p, a, b) <= w / 2.0);
    }
}

fn paint_shape(f: &mut Frame, shape: &Shape, fg: Color32) {
    match shape {
        Shape::Vec(v) => v.iter().for_each(|s| paint_shape(f, s, fg)),
        Shape::Rect(r) => {
            if r.fill != Color32::TRANSPARENT {
                fill_rect(f, r.rect, level(r.fill, fg));
            }
            let w = r.stroke.width;
            if w > 0.0 && r.stroke.color != Color32::TRANSPARENT {
                let w = w.max(1.0);
                let (outer, inner) = match r.stroke_kind {
                    StrokeKind::Inside => (r.rect, r.rect.shrink(w)),
                    StrokeKind::Middle => (r.rect.expand(w / 2.0), r.rect.shrink(w / 2.0)),
                    StrokeKind::Outside => (r.rect.expand(w), r.rect),
                };
                stroke_rect(f, outer, inner, level(r.stroke.color, fg));
            }
        }
        Shape::LineSegment { points, stroke } => {
            line(
                f,
                points[0],
                points[1],
                stroke.width,
                level(stroke.color, fg),
            );
        }
        Shape::Path(p) => {
            let ColorMode::Solid(color) = p.stroke.color else {
                return;
            };
            let lvl = level(color, fg);
            for w in p.points.windows(2) {
                line(f, w[0], w[1], p.stroke.width, lvl);
            }
        }
        Shape::Circle(c) => {
            let (center, radius) = (c.center, c.radius);
            let bounds = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
            fill_by(f, bounds, level(c.fill, fg), |p| {
                (p - center).length() <= radius
            });
        }
        _ => {}
    }
}

thread_local! {
    /// A private context that only runs the painters (never shown).
    static CTX: egui::Context = egui::Context::default();
}

/// Runs `body` with a painter and a 128 x 64 rectangle (one point per display pixel) and returns
/// the coverage of what it painted. `fg` is the colour of a fully lit pixel.
pub fn render(fg: Color32, body: impl FnOnce(&Painter, Rect)) -> Frame {
    let mut body = Some(body);
    let mut origin = Pos2::ZERO;
    let out = CTX.with(|ctx| {
        ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(W as f32, H as f32), Sense::hover());
                    origin = rect.min;
                    if let Some(f) = body.take() {
                        f(&ui.painter().clone(), rect);
                    }
                });
        })
    });
    let mut frame = vec![0.0; W * H];
    for s in &out.shapes {
        // Shapes are in the context's coordinates; move the display to (0, 0).
        let mut shape = s.shape.clone();
        shape.translate(-origin.to_vec2());
        paint_shape(&mut frame, &shape, fg);
    }
    frame
}

/// A texture kept between frames and the frame it shows.
#[derive(Clone)]
struct Cached {
    hash: u64,
    nearest: bool,
    texture: TextureHandle,
}

fn hash_of(frame: &Frame) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    frame.iter().for_each(|v| v.to_bits().hash(&mut h));
    h.finish()
}

/// White with the coverage as alpha (premultiplied), tinted by the caller.
fn image_of(frame: &Frame) -> ColorImage {
    let bytes: Vec<u8> = frame
        .iter()
        .flat_map(|v| {
            let a = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            [a, a, a, a]
        })
        .collect();
    ColorImage::from_rgba_premultiplied([W, H], &bytes)
}

/// Shows `frame` over `rect`. `id` names the display (one texture each).
pub fn show(painter: &Painter, rect: Rect, id: &str, fg: Color32, frame: &Frame) {
    let ctx = painter.ctx();
    let key = Id::new(("oled_fb", id));
    let hash = hash_of(frame);
    let nearest = rect.width() / W as f32 * ctx.pixels_per_point() >= NEAREST_FROM_CELL;
    let options = if nearest {
        TextureOptions::NEAREST
    } else {
        TextureOptions::LINEAR
    };
    let cached = match ctx.data(|d| d.get_temp::<Cached>(key)) {
        Some(mut c) => {
            if c.hash != hash || c.nearest != nearest {
                c.texture.set(image_of(frame), options);
                c.hash = hash;
                c.nearest = nearest;
                ctx.data_mut(|d| d.insert_temp(key, c.clone()));
            }
            c
        }
        None => {
            let c = Cached {
                hash,
                nearest,
                texture: ctx.load_texture(format!("oled_fb_{id}"), image_of(frame), options),
            };
            ctx.data_mut(|d| d.insert_temp(key, c.clone()));
            c
        }
    };
    painter.image(
        cached.texture.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        fg,
    );
}

/// Paints `body` (laid out on a 128 x 64 rectangle) into `rect` through the frame buffer when the
/// firmware fonts are installed, else calls it directly on `rect` (monospace fallback).
pub fn paint(
    painter: &Painter,
    rect: Rect,
    id: &str,
    fg: Color32,
    body: impl FnOnce(&Painter, Rect),
) {
    if nord_core::oled_font::font(0).is_none() {
        body(painter, rect);
        return;
    }
    show(painter, rect, id, fg, &render(fg, body));
}

#[cfg(test)]
mod tests {
    use super::*;

    const FG: Color32 = Color32::from_rgb(150, 190, 230);

    fn at(f: &Frame, x: usize, y: usize) -> f32 {
        f[y * W + x]
    }

    #[test]
    fn a_filled_rectangle_lights_exactly_its_pixels() {
        let f = render(FG, |p, r| {
            p.rect_filled(
                Rect::from_min_size(r.min + vec2(2.0, 3.0), vec2(4.0, 2.0)),
                0.0,
                FG,
            );
        });
        assert_eq!(at(&f, 2, 3), 1.0);
        assert_eq!(at(&f, 5, 4), 1.0);
        assert_eq!(at(&f, 6, 4), 0.0);
        assert_eq!(at(&f, 2, 5), 0.0);
        assert_eq!(f.iter().filter(|v| **v > 0.0).count(), 8);
    }

    #[test]
    fn an_edge_inside_a_pixel_gives_partial_coverage() {
        let f = render(FG, |p, r| {
            p.rect_filled(Rect::from_min_size(r.min, vec2(1.5, 1.0)), 0.0, FG);
        });
        assert_eq!(at(&f, 0, 0), 1.0);
        assert!((at(&f, 1, 0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_later_unlit_rectangle_cuts_a_hole() {
        let f = render(FG, |p, r| {
            p.rect_filled(r, 0.0, FG);
            p.rect_filled(
                Rect::from_min_size(r.min + vec2(10.0, 10.0), vec2(3.0, 3.0)),
                0.0,
                OLED_BG,
            );
        });
        assert_eq!(at(&f, 9, 10), 1.0);
        assert_eq!(at(&f, 11, 11), 0.0);
    }

    #[test]
    fn a_dim_colour_is_a_dim_level() {
        let f = render(FG, |p, r| {
            p.rect_filled(
                Rect::from_min_size(r.min, vec2(1.0, 1.0)),
                0.0,
                FG.gamma_multiply(0.4),
            );
        });
        let v = at(&f, 0, 0);
        assert!(v > 0.2 && v < 0.6, "{v}");
    }

    #[test]
    fn a_horizontal_rule_is_one_full_pixel_row() {
        let f = render(FG, |p, r| {
            p.line_segment(
                [r.min + vec2(1.0, 11.0), r.min + vec2(10.0, 11.0)],
                Stroke::new(0.5_f32, FG),
            );
        });
        assert_eq!(at(&f, 5, 11), 1.0);
        assert_eq!(at(&f, 5, 10), 0.0);
        assert_eq!(at(&f, 0, 11), 0.0);
    }

    #[test]
    fn an_inside_stroke_is_a_one_pixel_ring() {
        let f = render(FG, |p, r| {
            p.rect_stroke(
                Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(6.0, 6.0)),
                0.0,
                Stroke::new(0.7_f32, FG),
                StrokeKind::Inside,
            );
        });
        assert_eq!(at(&f, 4, 6), 1.0);
        assert_eq!(at(&f, 9, 6), 1.0);
        assert_eq!(at(&f, 6, 6), 0.0, "the inside stays dark");
    }

    #[test]
    fn a_diagonal_line_and_a_circle_are_partly_covered() {
        let f = render(FG, |p, r| {
            p.line_segment(
                [r.min + vec2(0.0, 0.0), r.min + vec2(20.0, 20.0)],
                Stroke::new(1.0_f32, FG),
            );
            p.circle_filled(r.min + vec2(40.0, 40.0), 3.0, FG);
        });
        assert!(at(&f, 10, 10) > 0.5);
        assert_eq!(at(&f, 10, 5), 0.0);
        assert_eq!(at(&f, 40, 40), 1.0);
        assert_eq!(at(&f, 40, 30), 0.0);
    }

    #[test]
    fn show_makes_one_texture_per_display_and_updates_it_on_change() {
        let ctx = egui::Context::default();
        let mut a = vec![0.0; W * H];
        a[0] = 1.0;
        let b = vec![0.0; W * H];
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let p = ui.painter().clone();
                let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(256.0, 128.0));
                show(&p, r, "t", FG, &a);
                let first = ctx
                    .data(|d| d.get_temp::<Cached>(Id::new(("oled_fb", "t"))))
                    .unwrap();
                show(&p, r, "t", FG, &b);
                let second = ctx
                    .data(|d| d.get_temp::<Cached>(Id::new(("oled_fb", "t"))))
                    .unwrap();
                assert_eq!(first.texture.id(), second.texture.id());
                assert_ne!(first.hash, second.hash);
            });
        });
    }

    #[test]
    fn without_the_firmware_fonts_the_body_runs_on_the_real_rectangle() {
        // The test binary installs no fonts unless the env var is set; skip when it did.
        if nord_core::oled_font::font(0).is_some() {
            return;
        }
        let ctx = egui::Context::default();
        let mut seen = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = Rect::from_min_size(pos2(5.0, 6.0), vec2(300.0, 150.0));
                paint(&ui.painter().clone(), r, "x", FG, |_, rect| {
                    seen = Some(rect)
                });
                assert_eq!(seen, Some(r));
            });
        });
    }
}
