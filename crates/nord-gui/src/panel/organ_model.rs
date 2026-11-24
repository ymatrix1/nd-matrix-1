//! The ORGAN MODEL LED block: six LEDs in two columns of three with the model names beside them.
//!
//! VERIFIED (presentation only): the printed order is FARF / PIPE1, VOX / PIPE2, B3 / B3 BASS
//! from top to bottom (photos `NS4_Panel-crop-left.jpg`, `NS4_HA88_Organ-01_241008.jpg`, local
//! reference; manual p.18 lists the same six). The model button still steps through
//! [`super::organ_logic::MODELS`] (engine order B3, B3 Bass, Vox, Farf, Pipe 1, Pipe 2); only the
//! place of each LED changes, via [`SLOTS`].

use super::ctl::led_small;
use super::prelude::*;

/// Printed slot (row * 2 + column) to index into `organ_logic::MODELS`, and the printed name.
pub(super) const SLOTS: [(usize, &str); 6] = [
    (3, "FARF"),
    (4, "PIPE1"),
    (2, "VOX"),
    (5, "PIPE2"),
    (0, "B3"),
    (1, "BASS"),
];

/// Centre of printed slot `slot` inside the LED rectangle `leds`.
pub(super) fn slot_pos(leds: Rect, slot: usize) -> egui::Pos2 {
    pos2(
        leds.left() + leds.width() * if slot.is_multiple_of(2) { 0.2 } else { 0.8 },
        leds.top() + leds.height() * ((slot / 2) as f32 + 0.5) / 3.0,
    )
}

/// Paint the LEDs and names; `lit` is the active index into `MODELS`.
pub(super) fn paint(painter: &Painter, leds: Rect, lit: usize) {
    for (slot, &(model, name)) in SLOTS.iter().enumerate() {
        let c = slot_pos(leds, slot);
        led_small(painter, c, model == lit, LED_ACCENT, LED_ACCENT_DIM, 1.5);
        let (x, al) = if slot.is_multiple_of(2) {
            (leds.left() - 3.0, Align2::RIGHT_CENTER)
        } else {
            (leds.right() + 3.0, Align2::LEFT_CENTER)
        };
        label(painter, pos2(x, c.y), name, 5.0, Color32::WHITE, al);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::organ_logic::MODELS;

    #[test]
    fn every_model_has_exactly_one_slot() {
        let mut seen = [false; 6];
        for &(m, _) in &SLOTS {
            assert!(!seen[m]);
            seen[m] = true;
        }
        assert!(seen.iter().all(|&s| s));
        assert_eq!(MODELS.len(), SLOTS.len());
    }

    #[test]
    fn printed_order_matches_the_photo() {
        let names: Vec<_> = SLOTS.iter().map(|s| s.1).collect();
        assert_eq!(names, ["FARF", "PIPE1", "VOX", "PIPE2", "B3", "BASS"]);
        // Left column reads FARF, VOX, B3 top to bottom.
        assert_eq!(SLOTS[0].1, "FARF");
        assert_eq!(SLOTS[2].1, "VOX");
        assert_eq!(SLOTS[4].1, "B3");
    }

    #[test]
    fn slots_are_two_columns_of_three() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(26.0, 40.0));
        assert!(slot_pos(r, 0).x < slot_pos(r, 1).x);
        assert!(slot_pos(r, 0).y < slot_pos(r, 2).y);
        assert!((slot_pos(r, 1).y - slot_pos(r, 0).y).abs() < 1e-4);
    }

    #[test]
    fn paints_leds_and_names() {
        let n = crate::panel::support::shapes_painted(|_, p, rect, _, _| {
            paint(p, Rect::from_min_size(rect.min, vec2(26.0, 40.0)), 4)
        });
        assert!(n >= 18, "{n}");
    }
}
