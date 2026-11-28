//! A name printed above a button with its state LED on the same line, left of the text, as the
//! real panel prints "o ORGAN" / "o STOP MODE" (photos `NS4_Panel-crop-left.jpg`,
//! `NS4_HA88_Organ-01_241008.jpg`). The text is centred over the button; the LED hangs at its left.

use super::ctl::{led_small, LEGEND_LED_GAP};
use super::prelude::*;

/// Lift (px) of the text above the button top: clears the bezel (1.5 px) and the audit margin.
pub(super) const LIFT: f32 = 3.5;
/// Radius (px) of the LED.
pub(super) const LED_R: f32 = 1.5;

/// Name `text` centred over `button`; the red state LED `lit` sits `LEGEND_LED_GAP` left of it.
/// Returns the text rectangle.
pub(super) fn tag_led(p: &Painter, button: Rect, text: &str, lit: bool) -> Rect {
    let at = pos2(button.center().x, button.top() - LIFT);
    let drawn = label(
        p,
        at,
        text,
        5.5,
        Color32::from_gray(235),
        Align2::CENTER_BOTTOM,
    );
    let c = pos2(drawn.left() - LEGEND_LED_GAP, drawn.center().y);
    led_small(p, c, lit, LED_ACCENT, LED_ACCENT_DIM, LED_R);
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_centred_on_the_button_and_the_led_is_left_of_it() {
        let btn = Rect::from_min_size(pos2(100.0, 100.0), vec2(37.0, 11.0));
        let mut r = Rect::NOTHING;
        crate::panel::support::shapes_painted(|_, p, _, _, _| r = tag_led(p, btn, "ORGAN", true));
        assert!((r.center().x - btn.center().x).abs() < 1.0, "{r:?}");
        assert!(r.bottom() <= btn.top() - 1.5, "{r:?}");
    }
}
