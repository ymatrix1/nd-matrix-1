//! The VIB/CHORUS type matrix of the Organ section: six LEDs in two rows with the type names
//! printed around them (`C2 V3 C3` above, `V2 C1 V1` below), as on the instrument.

use super::ctl::led_small;
use super::prelude::*;
use crate::widgets::label;

/// Column pitch (px) of the matrix.
const PITCH_X: f32 = 11.5;
/// Row pitch (px) between the two LED rows.
const PITCH_Y: f32 = 7.5;
/// Gap (px) between the selector button and the first column.
const GAP: f32 = 2.0;

/// Printed names, top row then bottom row (photo `NS4_Panel-crop-left.jpg`).
pub(super) const NAMES: [[&str; 3]; 2] = [["C2", "V3", "C3"], ["V2", "C1", "V1"]];

/// `(row, column)` of the LED for the type at `index` in the selector order `V1, C1, V2, C2, V3,
/// C3`: the printed matrix is that sequence as a snake from the bottom right.
/// VERIFIED: positions read from `NS4_Panel-crop-left.jpg` (local reference photo); checked by
/// `snake_matches_the_printed_names` below, which maps each position to its printed name.
pub(super) fn led_pos(index: usize) -> (usize, usize) {
    match index {
        0 => (1, 2),
        1 => (1, 1),
        2 => (1, 0),
        3 => (0, 0),
        4 => (0, 1),
        _ => (0, 2),
    }
}

/// Centre of the LED at `(row, col)` for the selector button `sel`.
pub(super) fn led_center(sel: Rect, row: usize, col: usize) -> egui::Pos2 {
    pos2(
        sel.right() + GAP + PITCH_X * (col as f32 + 0.5),
        sel.center().y + PITCH_Y * (row as f32 - 0.5),
    )
}

/// Paint the six LEDs (the one at selector index `cur` lit) and their names.
pub(super) fn paint(p: &Painter, sel: Rect, cur: usize) {
    let lit = led_pos(cur);
    for (row, names) in NAMES.iter().enumerate() {
        for (col, name) in names.iter().enumerate() {
            let c = led_center(sel, row, col);
            led_small(p, c, (row, col) == lit, LED_ACCENT, LED_ACCENT_DIM, 1.5);
            let (at, al) = if row == 0 {
                (pos2(c.x, c.y - 4.0), Align2::CENTER_BOTTOM)
            } else {
                (pos2(c.x, c.y + 4.0), Align2::CENTER_TOP)
            };
            label(p, at, name, 5.5, Color32::from_gray(235), al);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::organ_logic::{vib_index, VIB_TYPES};
    use nord_core::organ::ScannerMode;

    #[test]
    fn snake_matches_the_printed_names() {
        let expect = ["V1", "C1", "V2", "C2", "V3", "C3"];
        for (i, want) in expect.iter().enumerate() {
            let (r, c) = led_pos(i);
            assert_eq!(NAMES[r][c], *want, "LED {i}");
        }
        let mut seen: Vec<_> = (0..6).map(led_pos).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 6, "every LED position is used once");
    }

    #[test]
    fn selector_order_is_the_stored_vibrato_order() {
        assert_eq!(VIB_TYPES[0], ScannerMode::V1);
        assert_eq!(VIB_TYPES[1], ScannerMode::C1);
        assert_eq!(vib_index(ScannerMode::V3), 4);
    }

    #[test]
    fn led_centers_sit_right_of_the_button_in_two_rows() {
        let sel = Rect::from_min_size(pos2(100.0, 50.0), vec2(24.0, 13.0));
        let a = led_center(sel, 0, 0);
        let b = led_center(sel, 1, 2);
        assert!(a.x > sel.right() && b.x > a.x && b.y > a.y);
    }

    #[test]
    fn paint_draws_leds_and_names() {
        let n = crate::panel::support::shapes_painted(|_, p, r, _, _| {
            paint(p, Rect::from_min_size(r.min, vec2(24.0, 13.0)), 0)
        });
        assert!(n >= 12, "painted only {n} shapes");
    }
}
