//! The slim bar above the panel: `OPEN .NS4B`, `DEBUG`, `? HELP` pills, like the standalone app's
//! floating bar (the host owns the window, so there are no minimise / close buttons).

use nih_plug_egui::egui::{self, Button, Color32, CursorIcon, RichText, Stroke, Ui};

/// Height of the bar in logical points (the standalone bar is 22 px).
pub const BAR_HEIGHT: f32 = 22.0;

/// Which pill was clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarClick {
    OpenNs4b,
    Debug,
    Help,
}

/// Pills in drawing order.
pub const PILLS: [(BarClick, &str); 3] = [
    (BarClick::OpenNs4b, "OPEN .NS4B"),
    (BarClick::Debug, "DEBUG"),
    (BarClick::Help, "? HELP"),
];

fn pill(ui: &mut Ui, label: &str) -> egui::Response {
    let text = RichText::new(label)
        .size(9.0)
        .strong()
        .color(Color32::from_rgb(236, 240, 244));
    ui.add(
        Button::new(text)
            .fill(Color32::from_rgba_unmultiplied(20, 22, 26, 200))
            .stroke(Stroke::new(
                0.8_f32,
                Color32::from_rgba_unmultiplied(255, 255, 255, 45),
            ))
            .corner_radius(4.0),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
}

/// Draw the pills left to right; returns the one clicked this frame.
pub fn show(ui: &mut Ui) -> Option<BarClick> {
    let mut clicked = None;
    ui.spacing_mut().item_spacing.x = 5.0;
    ui.spacing_mut().button_padding = egui::vec2(7.0, 1.5);
    ui.horizontal_centered(|ui| {
        ui.add_space(4.0);
        for (which, label) in PILLS {
            if pill(ui, label).clicked() {
                clicked = Some(which);
            }
        }
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pills_match_the_standalone_bar() {
        let labels: Vec<_> = PILLS.iter().map(|p| p.1).collect();
        assert_eq!(labels, ["OPEN .NS4B", "DEBUG", "? HELP"]);
    }

    #[test]
    fn the_bar_draws_without_a_click() {
        let ctx = egui::Context::default();
        let mut got = Some(BarClick::Help);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| got = show(ui));
        });
        assert_eq!(got, None);
    }
}
