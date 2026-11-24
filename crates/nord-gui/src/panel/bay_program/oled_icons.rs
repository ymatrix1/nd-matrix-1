//! Icon column of the Program home rows: our own 8 x 8 px marks drawn as shapes.
//!
//! VERIFIED (frames of `tools/arm_screens.py`, `docs/research/arm-oled-render.md` section 12): each
//! section row of the real home screen starts with an 8 px icon at x = 0 to 7 and its text at
//! x = 12; a missing sound shows a warning glyph (`0x8A` of the 8 px font) before `Piano not found`.
//! The firmware bitmaps are not copied: these are simple stand-ins (ASSUMED shapes).

use crate::panel::prelude::*;
use egui::epaint::Shape;

/// Organ row icon (in-band marker, like [`super::oled_home::EDITED`]).
pub const ICON_ORGAN: char = '\u{E0E2}';
/// Piano row icon.
pub const ICON_PIANO: char = '\u{E0E3}';
/// Synth row icon.
pub const ICON_SYNTH: char = '\u{E0E4}';
/// Warning glyph of a missing sound.
pub const ICON_WARN: char = '\u{E0E5}';

/// List icon at the left of the title bar of the program lists.
pub const ICON_LIST: char = '\u{E0E6}';

/// Whether `c` is one of the icon markers.
pub fn is_icon(c: char) -> bool {
    matches!(
        c,
        ICON_ORGAN | ICON_PIANO | ICON_SYNTH | ICON_WARN | ICON_LIST
    )
}

/// Left edge of the row text when the row starts with an icon, as a fraction of the display
/// width (12 of 128 px).
pub const TEXT_X: f32 = 12.0 / 128.0;

/// Organ row icon, 8 x 8 px, one byte per row, bit 7 = left.
///
/// EMPIRICAL: our transcription of the 8 x 8 icon the ARM frame shows at the start of the organ
/// row (`tools/arm_screens.py press ... --key 0x01,0,3`, rows 36 to 42 of display 0, read from the
/// ASCII dump); not a copy of firmware bytes. Fit: pixel for pixel on that frame.
const ORGAN: [u8; 8] = [
    0b1101_1011,
    0b1101_0001,
    0b1101_0001,
    0b1101_1111,
    0b1000_1111,
    0b1000_1111,
    0b1111_1111,
    0,
];
/// Piano row icon (VERIFIED by the ARM pixel check: rows 46 to 52, five key rows, two solid).
const PIANO: [u8; 8] = [
    0b1010_1101,
    0b1010_1101,
    0b1010_1101,
    0b1010_1101,
    0b1010_1101,
    0b1111_1111,
    0b1111_1111,
    0,
];
/// Synth row icon (same source; rows 56 to 62, a ramp in a box).
const SYNTH: [u8; 8] = [
    0b1111_1111,
    0b1001_1111,
    0b1010_1111,
    0b1011_0111,
    0b1011_1011,
    0b1011_1101,
    0b1111_1111,
    0,
];

/// Draw `icon` in the square `cell` (side = 8 px of the real grid).
pub fn paint(painter: &Painter, cell: Rect, icon: char, color: Color32) {
    let stroke = Stroke::new(0.7_f32, color);
    let (l, t, w, h) = (cell.left(), cell.top(), cell.width(), cell.height());
    let at = |x: f32, y: f32| pos2(l + x * w, t + y * h);
    match icon {
        ICON_ORGAN | ICON_PIANO | ICON_SYNTH => {
            let rows = match icon {
                ICON_ORGAN => &ORGAN,
                ICON_PIANO => &PIANO,
                _ => &SYNTH,
            };
            // One filled square per lit pixel; edges rounded so neighbours leave no seam.
            let px = w / 8.0;
            let edge = |o: f32, i: usize| (o + i as f32 * px).round();
            for (y, row) in rows.iter().enumerate() {
                for x in 0..8 {
                    if (row >> (7 - x)) & 1 == 1 {
                        painter.rect_filled(
                            Rect::from_min_max(
                                pos2(edge(l, x), edge(t, y)),
                                pos2(edge(l, x + 1), edge(t, y + 1)),
                            ),
                            0.0,
                            color,
                        );
                    }
                }
            }
        }
        ICON_WARN => {
            painter.add(Shape::closed_line(
                vec![at(0.5, 0.05), at(1.0, 0.95), at(0.0, 0.95)],
                stroke,
            ));
            painter.line_segment([at(0.5, 0.35), at(0.5, 0.65)], stroke);
        }
        ICON_LIST => {
            // Three rows: a dot and a line each.
            for y in [0.15_f32, 0.5, 0.85] {
                painter.circle_filled(at(0.1, y), 0.07 * w, color);
                painter.line_segment([at(0.35, y), at(1.0, y)], stroke);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::support::shapes_painted;

    #[test]
    fn every_icon_is_a_marker_and_paints_shapes_only() {
        for icon in [ICON_ORGAN, ICON_PIANO, ICON_SYNTH, ICON_WARN, ICON_LIST] {
            assert!(is_icon(icon));
            let n = shapes_painted(|_ui, p, rect, _s, _a| {
                paint(
                    p,
                    Rect::from_min_size(rect.min, vec2(8.0, 8.0)),
                    icon,
                    Color32::WHITE,
                );
            });
            let base = shapes_painted(|_ui, _p, _rect, _s, _a| {});
            assert!(n > base, "{icon:?} painted {n} over {base}");
        }
        assert!(!is_icon('A'));
        let base = shapes_painted(|_ui, _p, _rect, _s, _a| {});
        let none = shapes_painted(|_ui, p, rect, _s, _a| {
            paint(
                p,
                Rect::from_min_size(rect.min, vec2(8.0, 8.0)),
                'A',
                Color32::WHITE,
            );
        });
        assert_eq!(none, base);
    }

    #[test]
    fn the_row_icons_have_the_lit_pixels_of_the_arm_frame() {
        let lit = |rows: &[u8; 8]| rows.iter().map(|r| r.count_ones()).sum::<u32>();
        // counted on the ASCII dump of the real frame: 39, 41 and 46 lit pixels
        assert_eq!((lit(&ORGAN), lit(&PIANO), lit(&SYNTH)), (39, 41, 46));
        assert_eq!(ORGAN[7] | PIANO[7] | SYNTH[7], 0, "unused rows stay dark");
    }

    #[test]
    fn the_row_text_starts_at_twelve_of_128_px() {
        assert_eq!(TEXT_X * 128.0, 12.0);
    }
}
