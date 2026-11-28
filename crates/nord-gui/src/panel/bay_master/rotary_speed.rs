//! The SLOW/STOP - FAST button of the Rotary Speaker box: its legend and the two speed LEDs.
//!
//! The row "o SLOW  o FAST" is centred on the frame's vertical centre line above the button, as
//! printed on the instrument (photo `NS4_Panel-crop-left.jpg`); the legend under the button is
//! centred on the same line.

use super::super::ctl::{gesture_button, led_small, mark, Face, LEGEND_LED_GAP};
use super::super::layout_px::rect_or_empty;
use super::super::prelude::*;
use super::rotary::send_rotary;
use crate::widgets::text_width;
use nord_panel::behaviour::Trigger;

/// Font size (px) of the speed names (raised to the minimum by `label`).
const NAME_SIZE: f32 = 5.0;
/// Space (px) between the end of "SLOW" and the LED of "FAST".
const PAIR_GAP: f32 = 7.0;
/// Radius (px) of the speed LEDs.
const LED_R: f32 = 1.6;

/// X of the first LED and of the second LED of the row centred on `cx`, given the two name
/// widths: `[LED gap name]  [LED gap name]` as one block.
pub(super) fn led_xs(cx: f32, w_slow: f32, w_fast: f32) -> (f32, f32) {
    let total = LEGEND_LED_GAP + w_slow + PAIR_GAP + LEGEND_LED_GAP + w_fast;
    let x0 = cx - total * 0.5;
    (x0, x0 + LEGEND_LED_GAP + w_slow + PAIR_GAP)
}

/// SLOW/STOP - FAST button, its two LEDs and legends.
pub(super) fn render_speed(
    ui: &mut Ui,
    painter: &Painter,
    o: egui::Pos2,
    frame: Rect,
    state: &mut Stage4PanelState,
    audio: &AudioHandle,
) {
    let fast = matches!(state.rotary_speed, RotarySpeed::Fast);
    let speed = rect_or_empty("rotary.speed", o);
    let cx = frame.center().x;
    // The row sits in its own layout rectangle (`rotary.speed_leds`, verified against the photo);
    // its names belong to it, so the label centres are inside that rectangle.
    let ly = rect_or_empty("rotary.speed_leds", o).center().y;
    let (ws, wf) = (
        text_width(painter, "SLOW", NAME_SIZE.max(crate::widgets::MIN_FONT_PX)),
        text_width(painter, "FAST", NAME_SIZE.max(crate::widgets::MIN_FONT_PX)),
    );
    let (lx_slow, lx_fast) = led_xs(cx, ws, wf);
    // LEDs show the real rotor state (a pedal can change it); `fast` stays the UI toggle.
    let (led_slow, led_fast) = (
        state.telemetry.rotary_slow(fast),
        state.telemetry.rotary_fast(fast),
    );
    led_small(
        painter,
        pos2(lx_slow, ly),
        led_slow,
        LED_GREEN,
        LED_GREEN_DIM,
        LED_R,
    );
    led_small(
        painter,
        pos2(lx_fast, ly),
        led_fast,
        LED_ACCENT,
        LED_ACCENT_DIM,
        LED_R,
    );
    for (x, name) in [(lx_slow, "SLOW"), (lx_fast, "FAST")] {
        label(
            painter,
            pos2(x + LEGEND_LED_GAP, ly),
            name,
            NAME_SIZE,
            Color32::WHITE,
            Align2::LEFT_CENTER,
        );
    }
    mark(ui, "rotary.speed_leds");
    let face = Face {
        tip: "Toggle slow/stop and fast rotor speed",
        ..Face::black()
    };
    if let Some(Trigger::Primary) = gesture_button(
        ui,
        &mut state.organ_ui.trackers,
        speed,
        "rotary.speed",
        false,
        face,
    ) {
        state.rotary_speed = if fast {
            RotarySpeed::Slow
        } else {
            RotarySpeed::Fast
        };
        send_rotary(state, audio);
    }
    label_fit(
        painter,
        pos2(cx, speed.bottom() + 6.5),
        "SLOW/STOP - FAST",
        5.0,
        frame.width() - 4.0,
        Color32::WHITE,
        Align2::CENTER_CENTER,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_led_row_is_centred_on_the_line() {
        let (a, b) = led_xs(100.0, 20.0, 20.0);
        let right_end = b + LEGEND_LED_GAP + 20.0;
        assert!(((a + right_end) * 0.5 - 100.0).abs() < 3.0, "{a} {b}");
        assert!(b > a + 20.0);
    }

    #[test]
    fn speed_button_and_row_paint() {
        let n = crate::panel::support::shapes_painted(|ui, p, rect, s, a| {
            let frame = Rect::from_min_size(rect.min, vec2(82.0, 150.0));
            render_speed(ui, p, rect.min, frame, s, a)
        });
        assert!(n > 8, "{n}");
    }
}
