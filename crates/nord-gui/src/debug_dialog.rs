//! Shadcn/ui-inspired separate-window Settings & Diagnostics dialog for `nord-stage4`.
//!
//! Opens in its own native OS viewport (`show_viewport_immediate`) so it is never clipped by the
//! compact Nord Stage 4 hardware window, while sharing the exact same borderless dark hardware
//! aesthetic and vector Minimize / Close icon buttons.

use crate::audio::AudioHandle;
use crate::firmware_panel::FirmwarePanelState;
use crate::library::BackupLibraryState;
use crate::os_update_panel::OsUpdatePanel;
use crate::panel::Stage4PanelState;
use egui::{
    Color32, Context, CornerRadius, Frame, Margin, RichText, Stroke, Ui, ViewportBuilder,
    ViewportClass, ViewportCommand, ViewportId,
};

const ZINC_950: Color32 = Color32::from_rgb(9, 9, 11);
const ZINC_900: Color32 = Color32::from_rgb(24, 24, 27);
const ZINC_800: Color32 = Color32::from_rgb(39, 39, 42);
const ZINC_400: Color32 = Color32::from_rgb(161, 161, 170);
const ZINC_50: Color32 = Color32::from_rgb(250, 250, 250);
const ACCENT_RED: Color32 = Color32::from_rgb(184, 37, 42);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DebugTab {
    #[default]
    Firmware,
    AudioMidi,
    Library,
    OsUpdate,
}

impl DebugTab {
    pub const ALL: [Self; 4] = [
        Self::Firmware,
        Self::AudioMidi,
        Self::Library,
        Self::OsUpdate,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Firmware => "Firmware & DSP",
            Self::AudioMidi => "Audio & MIDI",
            Self::Library => "Library & Backup",
            Self::OsUpdate => "OS & Wave Tables",
        }
    }
}

/// Persistent UI state for the separate-window shadcn-styled Diagnostics & Settings dialog.
#[derive(Default)]
pub struct DebugDialogState {
    pub open: bool,
    pub tab: DebugTab,
}

fn shadcn_card(ui: &mut Ui, title: &str, subtitle: &str, body: impl FnOnce(&mut Ui)) {
    Frame::NONE
        .fill(ZINC_900)
        .stroke(Stroke::new(1.0, ZINC_800))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(title).size(13.0).strong().color(ZINC_50));
            ui.label(RichText::new(subtitle).size(11.0).color(ZINC_400));
            ui.add_space(8.0);
            body(ui);
        });
}

fn paint_window_controls(ctx: &Context, ui: &mut Ui, dlg: &mut DebugDialogState) {
    // Vector Close button (rightmost in right_to_left layout)
    let (cls_rect, cls_resp) = ui.allocate_exact_size(egui::vec2(24.0, 18.0), egui::Sense::click());
    let cls_resp = cls_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let cls_bg = if cls_resp.hovered() {
        ACCENT_RED
    } else {
        ZINC_900
    };
    ui.painter().rect_filled(cls_rect, 4.0, cls_bg);
    ui.painter().rect_stroke(
        cls_rect,
        4.0,
        Stroke::new(0.8, ZINC_800),
        egui::StrokeKind::Inside,
    );
    let c = cls_rect.center();
    let d = 4.0;
    let stroke = Stroke::new(1.5, ZINC_50);
    ui.painter()
        .line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], stroke);
    ui.painter()
        .line_segment([c + egui::vec2(d, -d), c + egui::vec2(-d, d)], stroke);
    if cls_resp.clicked() {
        dlg.open = false;
    }

    // Vector Minimize button
    let (min_rect, min_resp) = ui.allocate_exact_size(egui::vec2(24.0, 18.0), egui::Sense::click());
    let min_resp = min_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let min_bg = if min_resp.hovered() {
        Color32::from_rgb(46, 50, 58)
    } else {
        ZINC_900
    };
    ui.painter().rect_filled(min_rect, 4.0, min_bg);
    ui.painter().rect_stroke(
        min_rect,
        4.0,
        Stroke::new(0.8, ZINC_800),
        egui::StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [
            egui::pos2(min_rect.center().x - 4.5, min_rect.center().y + 1.5),
            egui::pos2(min_rect.center().x + 4.5, min_rect.center().y + 1.5),
        ],
        Stroke::new(1.5, ZINC_50),
    );
    if min_resp.clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

fn render_dialog_contents(
    ui: &mut Ui,
    dlg: &mut DebugDialogState,
    state: &mut Stage4PanelState,
    library: &mut BackupLibraryState,
    firmware: &mut FirmwarePanelState,
    os_update: &mut OsUpdatePanel,
    audio_handle: &AudioHandle,
) {
    let ctx = ui.ctx().clone();
    // Header row (draggable window title + consistent vector Minimize / Close icons)
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new("Studio Settings & Diagnostics")
                    .size(16.0)
                    .strong()
                    .color(ZINC_50),
            );
            ui.label(
                RichText::new(
                    "Firmware DSP options, MIDI & audio metering, and .ns4b backup tools.",
                )
                .size(11.5)
                .color(ZINC_400),
            );
        });
        let drag_w = (ui.available_width() - 62.0).max(16.0);
        let (_, drag_resp) =
            ui.allocate_exact_size(egui::vec2(drag_w, 28.0), egui::Sense::click_and_drag());
        if drag_resp.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
        }
        if drag_resp.is_pointer_button_down_on() && drag_resp.dragged() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            paint_window_controls(&ctx, ui, dlg);
        });
    });

    ui.add_space(12.0);

    // Segmented pill tab bar (shadcn TabsList)
    Frame::NONE
        .fill(ZINC_900)
        .stroke(Stroke::new(1.0, ZINC_800))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for tab in DebugTab::ALL {
                    let active = dlg.tab == tab;
                    let fill = if active {
                        ACCENT_RED
                    } else {
                        Color32::TRANSPARENT
                    };
                    let text_col = if active { ZINC_50 } else { ZINC_400 };
                    let btn = ui
                        .add(
                            egui::Button::new(
                                RichText::new(tab.label())
                                    .size(11.5)
                                    .strong()
                                    .color(text_col),
                            )
                            .fill(fill)
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(6)),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if btn.clicked() {
                        dlg.tab = tab;
                    }
                }
            });
        });

    ui.add_space(12.0);

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| match dlg.tab {
            DebugTab::Firmware => {
                shadcn_card(
                    ui,
                    "Firmware & DSP Engine Options",
                    "Toggle ARM/SHARC firmware-derived synthesis, filter, and FX laws.",
                    |ui| {
                        firmware.show_bar(ui, audio_handle);
                    },
                );
            }
            DebugTab::AudioMidi => {
                shadcn_card(
                    ui,
                    "Computer Keyboard & Velocity",
                    "Play the Nord Stage 4 directly from your computer keyboard.",
                    |ui| {
                        crate::panel::computer_kb_bar::show_bar(ui, &mut state.computer_kb);
                    },
                );
                ui.add_space(10.0);
                shadcn_card(
                    ui,
                    "MIDI Input & Audio Engine Meter",
                    "Live MIDI port selection and real-time audio callback health.",
                    |ui| {
                        if let Some(ports) = audio_handle.midi_ports() {
                            crate::audio::midi_ports::show(ui, ports);
                            ui.add_space(6.0);
                        }
                        crate::audio::meter_ui::show(ui, audio_handle.meter());
                    },
                );
            }
            DebugTab::Library => {
                shadcn_card(
                    ui,
                    "Backup Library (.ns4b / .ns4p)",
                    "Open a Nord Stage 4 factory/user backup, sync knobs on preset load, or export.",
                    |ui| {
                        library.render_top_bar(ui, state);
                    },
                );
            }
            DebugTab::OsUpdate => {
                shadcn_card(
                    ui,
                    "Nord OS Update & Wave Tables",
                    "Load Nord OS update packages for authentic SHARC wave-bank tables.",
                    |ui| {
                        os_update.show_bar(ui);
                    },
                );
            }
        });
}

/// Render the separate-window shadcn-style Settings & Diagnostics dialog when `dlg.open` is true.
pub fn show_debug_dialog(
    ctx: &Context,
    dlg: &mut DebugDialogState,
    state: &mut Stage4PanelState,
    library: &mut BackupLibraryState,
    firmware: &mut FirmwarePanelState,
    os_update: &mut OsUpdatePanel,
    audio_handle: &AudioHandle,
) {
    if !dlg.open {
        return;
    }

    let viewport_id = ViewportId::from_hash_of("ns4_shadcn_debug_viewport");
    let builder = ViewportBuilder::default()
        .with_title("Nord Stage 4 — Studio Settings & Diagnostics")
        .with_decorations(false)
        .with_inner_size([880.0, 500.0])
        .with_min_inner_size([600.0, 340.0]);

    ctx.show_viewport_immediate(viewport_id, builder, |vp_ctx, class| {
        if vp_ctx.input(|i| i.viewport().close_requested()) {
            dlg.open = false;
            return;
        }
        let frame = Frame::NONE
            .fill(ZINC_950)
            .stroke(Stroke::new(1.0, ZINC_800))
            .inner_margin(Margin::same(16));

        if class == ViewportClass::Embedded {
            let mut open = dlg.open;
            egui::Window::new("Studio Settings & Diagnostics")
                .id(egui::Id::new("ns4_shadcn_debug_embedded"))
                .open(&mut open)
                .title_bar(false)
                .frame(frame.corner_radius(CornerRadius::same(10)))
                .show(vp_ctx, |ui| {
                    render_dialog_contents(
                        ui,
                        dlg,
                        state,
                        library,
                        firmware,
                        os_update,
                        audio_handle,
                    );
                });
            if !open {
                dlg.open = false;
            }
        } else {
            egui::CentralPanel::default()
                .frame(frame)
                .show(vp_ctx, |ui| {
                    render_dialog_contents(
                        ui,
                        dlg,
                        state,
                        library,
                        firmware,
                        os_update,
                        audio_handle,
                    );
                });

            // Bottom-right corner resize handle for the borderless settings window
            let screen = vp_ctx.screen_rect();
            let corner = egui::Rect::from_min_max(
                egui::pos2(screen.right() - 14.0, screen.bottom() - 14.0),
                screen.right_bottom(),
            );
            let (hovered, pressed) = vp_ctx.input(|i| {
                let h = i.pointer.hover_pos().is_some_and(|p| corner.contains(p));
                (h, h && i.pointer.primary_pressed())
            });
            if hovered {
                vp_ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
            if pressed {
                vp_ctx.send_viewport_cmd(ViewportCommand::BeginResize(
                    egui::ResizeDirection::SouthEast,
                ));
            }
        }
        os_update.show_window(vp_ctx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_dialog_renders_all_tabs_without_panics() {
        let ctx = Context::default();
        let (audio, _rx) = AudioHandle::new();
        let mut state = Stage4PanelState::default();
        let mut library = BackupLibraryState::new();
        let mut firmware = FirmwarePanelState::new(Default::default());
        let mut os_update = OsUpdatePanel::new(None, None);
        let mut dlg = DebugDialogState {
            open: true,
            tab: DebugTab::Firmware,
        };
        for tab in DebugTab::ALL {
            dlg.tab = tab;
            let _ = ctx.run(Default::default(), |ctx| {
                show_debug_dialog(
                    ctx,
                    &mut dlg,
                    &mut state,
                    &mut library,
                    &mut firmware,
                    &mut os_update,
                    &audio,
                );
            });
        }
    }
}
