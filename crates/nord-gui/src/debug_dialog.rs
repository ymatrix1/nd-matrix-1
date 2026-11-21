//! Shadcn/ui-inspired floating Settings & Diagnostics dialog for `nord-stage4`.
//!
//! Keeps the borderless transparent Nord Stage 4 hardware view uncluttered while giving
//! clean tabbed access to Firmware/DSP flags, Audio & MIDI, Backup Library, and OS Updates.

use crate::audio::AudioHandle;
use crate::firmware_panel::FirmwarePanelState;
use crate::library::BackupLibraryState;
use crate::os_update_panel::OsUpdatePanel;
use crate::panel::Stage4PanelState;
use egui::{Color32, Context, CornerRadius, Frame, Margin, RichText, Stroke, Ui};

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

/// Persistent UI state for the floating shadcn-styled Diagnostics & Settings dialog.
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

/// Render the floating shadcn-style Settings & Diagnostics dialog when `dlg.open` is true.
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

    let mut is_open = dlg.open;
    let window_frame = Frame::NONE
        .fill(ZINC_950)
        .stroke(Stroke::new(1.0, ZINC_800))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(16))
        .shadow(egui::epaint::Shadow {
            offset: [0, 12],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(170),
        });

    egui::Window::new("Nord Stage 4 — Studio Settings & Diagnostics")
        .id(egui::Id::new("ns4_shadcn_debug_dialog"))
        .open(&mut is_open)
        .title_bar(false)
        .resizable(true)
        .collapsible(false)
        .default_size([740.0, 440.0])
        .min_size([560.0, 320.0])
        .frame(window_frame)
        .show(ctx, |ui| {
            // Header row
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Studio Settings & Diagnostics")
                            .size(16.0)
                            .strong()
                            .color(ZINC_50),
                    );
                    ui.label(
                        RichText::new("Firmware DSP options, MIDI & audio metering, and .ns4b backup tools.")
                            .size(11.5)
                            .color(ZINC_400),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close_btn = ui
                        .add(
                            egui::Button::new(
                                RichText::new("Close").size(11.0).strong().color(ZINC_50),
                            )
                            .fill(ZINC_900)
                            .stroke(Stroke::new(1.0, ZINC_800))
                            .corner_radius(CornerRadius::same(6)),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if close_btn.clicked() {
                        dlg.open = false;
                    }
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
                            let fill = if active { ACCENT_RED } else { Color32::TRANSPARENT };
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

            egui::ScrollArea::vertical()
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
                                crate::panel::computer_kb_bar::show_bar(
                                    ui,
                                    &mut state.computer_kb,
                                );
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
        });

    os_update.show_window(ctx);
    if !is_open {
        dlg.open = false;
    }
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
