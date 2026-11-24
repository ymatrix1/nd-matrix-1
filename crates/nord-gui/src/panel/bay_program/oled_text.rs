//! OLED text: the firmware's own bitmap fonts when the user's Nord OS update file is set, our
//! monospace painter otherwise.
//!
//! D128: the glyph bitmaps are read at run time by `nord_core::oled_font` and never stored. The
//! two sizes of the layout map to the firmware fonts of `docs/research/arm-oled-render.md` s3 / s5:
//! the 13 px text font (index 0: title, name, values) and the 8 px font (index 3: rows, labels,
//! popups). Without fonts nothing changes: the caller's monospace size is used.

use crate::panel::prelude::*;
use egui::{FontId, Pos2};
use nord_core::oled_font::{self, Font};

/// UI font index of the 13 px text font (VERIFIED: title `A:11`, name line, menu values).
pub const BIG_FONT: usize = 0;
/// UI font index of the 8 px text font (VERIFIED: section rows, popup labels, menu labels).
pub const SMALL_FONT: usize = 3;
/// A line whose monospace size is at least this many display pixels is the 13 px font.
const BIG_FROM_PX: f32 = 10.0;

/// The font for a line of monospace `size` at `cell` screen pixels per display pixel.
pub fn font_for(size: f32, cell: f32) -> Option<Font> {
    let idx = if size / cell.max(0.01) >= BIG_FROM_PX {
        BIG_FONT
    } else {
        SMALL_FONT
    };
    oled_font::font(idx)
}

/// Screen width of `s` (bitmap font when given, else the monospace layout).
pub fn width(painter: &Painter, font: Option<&Font>, s: &str, size: f32, cell: f32) -> f32 {
    match font {
        Some(f) => f.text_width(s) as f32 * cell,
        None => {
            painter
                .layout_no_wrap(
                    s.to_string(),
                    FontId::monospace(size.max(MIN_FONT_PX)),
                    Color32::WHITE,
                )
                .size()
                .x
        }
    }
}

/// Draws `s` anchored at `at` like `Painter::text` and returns the rectangle it covers.
/// `size` is the monospace size (fallback and font choice), `cell` the screen size of one display
/// pixel.
pub fn draw(
    painter: &Painter,
    at: Pos2,
    anchor: Align2,
    s: &str,
    size: f32,
    cell: f32,
    color: Color32,
) -> Rect {
    draw_with(
        painter,
        font_for(size, cell).as_ref(),
        at,
        anchor,
        s,
        size,
        cell,
        color,
    )
}

/// [`draw`] with an explicit font (`None` = the monospace fallback).
#[allow(clippy::too_many_arguments)]
pub fn draw_with(
    painter: &Painter,
    font: Option<&Font>,
    at: Pos2,
    anchor: Align2,
    s: &str,
    size: f32,
    cell: f32,
    color: Color32,
) -> Rect {
    let Some(font) = font else {
        return painter.text(
            at,
            anchor,
            s,
            FontId::monospace(size.max(MIN_FONT_PX)),
            color,
        );
    };
    let (w, h) = (font.text_width(s) as f32 * cell, font.height as f32 * cell);
    let rect = anchor.anchor_size(at, vec2(w, h));
    // One filled rectangle per vertical run of lit pixels in a column.
    let mut x = 0;
    for c in s.chars().take_while(|c| *c != '\0' && *c != '\n') {
        for col in font.columns(c as u32) {
            let mut row = 0;
            while row < font.height.min(32) {
                if (col >> row) & 1 == 0 {
                    row += 1;
                    continue;
                }
                let start = row;
                while row < font.height.min(32) && (col >> row) & 1 == 1 {
                    row += 1;
                }
                // Edges are rounded to whole screen pixels so neighbouring runs leave no seam.
                let edge = |origin: f32, i: u32| (origin + i as f32 * cell).round();
                painter.rect_filled(
                    Rect::from_min_max(
                        pos2(edge(rect.left(), x), edge(rect.top(), start)),
                        pos2(edge(rect.left(), x + 1), edge(rect.top(), row)),
                    ),
                    0.0,
                    color,
                );
            }
            x += 1;
        }
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::support::shapes_painted;

    /// A 2 character font: `A` = column `0b101` then a gap, `B` = a full 3 row column.
    fn tiny() -> Font {
        Font {
            height: 3,
            first: b'A',
            last: b'B',
            glyphs: vec![vec![0b101, 0], vec![0b111]],
        }
    }

    fn base() -> usize {
        shapes_painted(|_ui, _p, _rect, _s, _a| {})
    }

    #[test]
    fn a_bitmap_font_paints_one_rectangle_per_vertical_run() {
        let n = shapes_painted(|_ui, p, rect, _s, _a| {
            let r = draw_with(
                p,
                Some(&tiny()),
                rect.min,
                Align2::LEFT_TOP,
                "AB",
                10.0,
                2.0,
                Color32::WHITE,
            );
            // width = 3 columns (the gap before B counts, the trailing one does not) x cell 2
            assert_eq!(r.width(), 6.0);
            assert_eq!(r.height(), 6.0);
        });
        // A: rows 0 and 2 are two runs, the gap none, B: one run
        assert_eq!(n - base(), 3);
    }

    #[test]
    fn without_a_font_the_monospace_fallback_paints_text() {
        let n = shapes_painted(|_ui, p, rect, _s, _a| {
            draw_with(
                p,
                None,
                rect.min,
                Align2::LEFT_TOP,
                "AB",
                10.0,
                1.0,
                Color32::WHITE,
            );
        });
        assert!(n > base());
    }

    #[test]
    fn anchors_place_the_bitmap_text_at_the_right_edge_and_centre() {
        shapes_painted(|_ui, p, rect, _s, _a| {
            let f = tiny();
            let at = pos2(rect.left() + 50.0, rect.top());
            let right = draw_with(
                p,
                Some(&f),
                at,
                Align2::RIGHT_TOP,
                "B",
                5.0,
                1.0,
                Color32::WHITE,
            );
            assert_eq!(right.right(), at.x);
            let mid = draw_with(
                p,
                Some(&f),
                at,
                Align2::CENTER_CENTER,
                "B",
                5.0,
                1.0,
                Color32::WHITE,
            );
            assert_eq!(mid.center(), at);
        });
    }

    #[test]
    fn the_big_size_picks_the_13_px_font_and_the_row_size_the_8_px_one() {
        // Whatever is installed, the choice is by size: 12.2 display px is big, 7.7 is small.
        let big = font_for(12.2, 1.0).map(|f| f.height);
        let small = font_for(7.7, 1.0).map(|f| f.height);
        if let (Some(b), Some(s)) = (big, small) {
            assert!(b > s);
        }
        assert_eq!(
            font_for(12.2, 2.0).map(|f| f.height),
            small,
            "size is scaled by the cell"
        );
    }
}
