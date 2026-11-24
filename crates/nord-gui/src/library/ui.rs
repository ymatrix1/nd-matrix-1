//! Library drawer and top-bar rendering.

use super::init_load::InitTarget;
use super::{BackupLibraryState, BackupTab};
use crate::audio::AudioHandle;
use crate::panel::Stage4PanelState;
use egui::{Color32, RichText};
use std::path::PathBuf;

impl BackupLibraryState {
    /// Renders the compact top library utility strip.
    pub fn render_top_bar(&mut self, ui: &mut egui::Ui, panel: &mut Stage4PanelState) {
        ui.horizontal(|ui| {
            let toggle_label = if self.drawer_open {
                "Close Library"
            } else {
                "Open .ns4b Library"
            };
            if ui.button(toggle_label).clicked() {
                self.drawer_open = !self.drawer_open;
                if self.drawer_open && self.archive.is_none() && !self.path_input.is_empty() {
                    let p = PathBuf::from(self.path_input.trim());
                    if p.is_file() {
                        if let Err(err) = self.open_backup_path(&p) {
                            self.error_message = Some(err);
                        }
                    }
                }
            }

            ui.separator();
            ui.checkbox(&mut panel.sync_knobs_on_program_load, "Sync Knobs on Preset")
                .on_hover_text(
                    "Real Nord Stage 4 knobs are non-motorized (OLED & LED ladders update on load, while pots keep their physical angle). Enable to move GUI knobs on preset load.",
                );

            ui.separator();
            ui.add_enabled_ui(self.backup_path.is_some(), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.export_path)
                        .desired_width(crate::text_scale::px(170.0))
                        .hint_text("Export path (.ns4b)"),
                );
                if ui
                    .button("Export backup")
                    .on_hover_text(
                        "Write the stored and organized programs and the Live slots into a copy of the opened .ns4b. The opened file is never changed.",
                    )
                    .clicked()
                {
                    let r = self.export_backup(&panel.program_ui.library);
                    self.report(r);
                }
                if ui
                    .button("Load Live slots")
                    .on_hover_text("Load the Live slots (.ns4l) of the opened backup into the eight Live programs.")
                    .clicked()
                {
                    let r = self.import_live_slots(&mut panel.program_ui.library);
                    self.report(r);
                }
            });

            ui.separator();
            if self.pending_jobs > 0 {
                ui.spinner();
            }
            if let Some(err) = &self.error_message {
                ui.colored_label(Color32::from_rgb(255, 135, 135), err);
            } else {
                ui.label(RichText::new(&self.status_message).color(Color32::LIGHT_GRAY));
            }
        });
    }

    /// Opens a dropped / command-line `.ns4b` and shows it in the Studio Settings "Library &
    /// Backup" tab (the current template), not the legacy docked drawer. On failure the error is
    /// kept in `error_message` and the dialog stays as it was.
    pub fn open_backup_in_settings(
        &mut self,
        path: &std::path::Path,
        dialog: &mut crate::debug_dialog::DebugDialogState,
    ) {
        match self.add_backup_path(path) {
            Ok(_) => {
                self.path_input = path.display().to_string();
                self.error_message = None;
                dialog.open_tab(crate::debug_dialog::DebugTab::Library);
            }
            Err(err) => self.error_message = Some(err),
        }
    }

    /// Shows the outcome of an Export / Live import in the status strip.
    fn report(&mut self, result: Result<String, String>) {
        match result {
            Ok(msg) => {
                self.status_message = msg;
                self.error_message = None;
            }
            Err(err) => self.error_message = Some(err),
        }
    }

    /// L (Shift+L while the Computer keyboard is on) toggles the library drawer (like Z / P for the view). Ignored while a text field has
    /// the keyboard. Returns `true` when it toggled.
    pub fn handle_keys(&mut self, ctx: &egui::Context) -> bool {
        if ctx.wants_keyboard_input() {
            return false;
        }
        let claimed = crate::panel::computer_keys::claimed(ctx);
        let l =
            ctx.input(|i| crate::panel::computer_keys::shortcut_pressed(i, egui::Key::L, claimed));
        if l {
            self.drawer_open = !self.drawer_open;
        }
        l
    }

    /// Renders the `.ns4b` archive browser controls and scrollable preset list inline
    /// (shared by the docked drawer and the separate Studio Settings dialog).
    pub fn render_browser_inline(
        &mut self,
        ui: &mut egui::Ui,
        panel: &mut Stage4PanelState,
        audio: &AudioHandle,
    ) {
        ui.horizontal(|ui| {
            ui.label("Backup (.ns4b):");
            let input_w = (ui.available_width() - crate::text_scale::px(156.0))
                .max(crate::text_scale::px(160.0));
            let field_h = ui.spacing().interact_size.y;
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.path_input)
                    .desired_width(input_w)
                    .min_size(egui::vec2(input_w, field_h))
                    .margin(egui::Margin::symmetric(8, 5))
                    .vertical_align(egui::Align::Center)
                    .hint_text("Path to .ns4b backup (or click Browse / drag-and-drop)"),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let browse_clicked = ui
                .button("Browse...")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(
                    "Open File Explorer to choose a .ns4b backup, .ns4p, .npno, or .nsmp4 file",
                )
                .clicked();
            let open_clicked = ui
                .button("Open")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();

            // The dialog runs on a worker thread; its answer arrives on a later frame.
            let from_dialog = crate::file_picker::poll_slot(&mut self.pending_pick, ui.ctx());
            let picked = if browse_clicked || (open_clicked && self.path_input.trim().is_empty()) {
                crate::file_picker::start_slot(
                    &mut self.pending_pick,
                    crate::file_picker::PickKind::BackupOrSound,
                );
                from_dialog
            } else if open_clicked || enter {
                let raw = self.path_input.trim().trim_matches('"');
                (!raw.is_empty()).then(|| PathBuf::from(raw))
            } else {
                from_dialog
            };

            if let Some(p) = picked {
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase());
                match ext.as_deref() {
                    Some("npno") => {
                        self.request_piano_file(p, self.piano_slot_b);
                    }
                    Some("nsmp4") => {
                        self.request_sample_file(p, self.synth_layer_idx);
                    }
                    Some("ns4p") => match std::fs::read(&p) {
                        Ok(bytes) => {
                            let parsed = nord_core::cbin::CbinFile::parse(&bytes)
                                .and_then(|c| nord_core::cbin::ProgramDecoded::from_cbin_file(&c));
                            match parsed {
                                Ok(mut prog) => {
                                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                                        if !stem.is_empty() {
                                            prog.name = stem.to_string();
                                        }
                                    }
                                    panel.apply_program(&prog);
                                    audio.send_command(crate::audio::AudioCommand::LoadProgram(
                                        Box::new(prog),
                                    ));
                                    self.error_message = None;
                                }
                                Err(err) => {
                                    self.error_message =
                                        Some(format!("Failed to parse .ns4p: {err:?}"));
                                }
                            }
                        }
                        Err(err) => {
                            self.error_message = Some(format!("Failed to read file: {err}"));
                        }
                    },
                    _ => {
                        self.path_input = p.display().to_string();
                        if let Err(err) = self.add_backup_path(&p) {
                            self.error_message = Some(err);
                        }
                    }
                }
            }
        });

        if let Some(err) = &self.error_message {
            ui.add_space(4.0);
            ui.label(
                RichText::new(err)
                    .small()
                    .color(Color32::from_rgb(248, 113, 113)),
            );
        }

        if let Some(ver) = &self.manifest_summary {
            ui.add_space(4.0);
            ui.small(format!(
                "Archive OS: {ver}  •  Programs: {}  •  Pianos: {}  •  Samples: {}",
                self.programs.len(),
                self.pianos.len(),
                self.samples.len()
            ));
        }
        if self.is_open() {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                let names: Vec<String> = self
                    .bank_paths()
                    .iter()
                    .map(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("bank")
                            .to_string()
                    })
                    .collect();
                ui.small(format!(
                    "Banks ({}): {}  •  Export / Organize use the first",
                    names.len(),
                    names.join(", ")
                ));
                if ui
                    .small_button("Close all")
                    .on_hover_text("Close every bank. Browse / Open / drag-and-drop adds more banks to the open ones.")
                    .clicked()
                {
                    self.close_banks();
                }
            });
        }

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.active_tab,
                BackupTab::Programs,
                format!("Programs ({})", self.programs.len()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.selectable_value(
                &mut self.active_tab,
                BackupTab::Pianos,
                format!("Pianos ({})", self.pianos.len()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.selectable_value(
                &mut self.active_tab,
                BackupTab::Samples,
                format!("Samples ({})", self.samples.len()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.separator();
            ui.checkbox(
                &mut panel.sync_knobs_on_program_load,
                "Sync Knobs on Preset",
            );
        });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Filter:");
            let field_h = ui.spacing().interact_size.y;
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .desired_width(crate::text_scale::px(280.0))
                    .min_size(egui::vec2(crate::text_scale::px(280.0), field_h))
                    .margin(egui::Margin::symmetric(8, 5))
                    .vertical_align(egui::Align::Center)
                    .hint_text("Search preset, piano, or sample..."),
            );
            if !self.filter.is_empty() && ui.button("Clear").clicked() {
                self.filter.clear();
            }
            if self.active_tab == BackupTab::Pianos {
                ui.label("Target:");
                ui.selectable_value(&mut self.piano_slot_b, false, "Piano A");
                ui.selectable_value(&mut self.piano_slot_b, true, "Piano B");
            } else if self.active_tab == BackupTab::Samples {
                ui.label("Target:");
                ui.selectable_value(&mut self.synth_layer_idx, 0, "Synth A");
                ui.selectable_value(&mut self.synth_layer_idx, 1, "Synth B");
                ui.selectable_value(&mut self.synth_layer_idx, 2, "Synth C");
            }
            if self.active_tab != BackupTab::Programs {
                ui.checkbox(&mut self.merge_into_program, "Merge into current program")
                    .on_hover_text(
                        "Off: the sound loads into a fresh Init Program (other sections, effects and levels reset). On: only this layer's sound changes.",
                    );
            }
        });

        ui.add_space(6.0);
        ui.separator();
        let query = self.filter.trim().to_ascii_lowercase();
        let mut selected_program: Option<String> = None;
        let mut selected_piano: Option<String> = None;
        let mut selected_sample: Option<String> = None;

        egui::ScrollArea::vertical()
            .max_height(240.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let items: &[String] = match self.active_tab {
                    BackupTab::Programs => &self.programs,
                    BackupTab::Pianos => &self.pianos,
                    BackupTab::Samples => &self.samples,
                };
                if items.is_empty() {
                    ui.weak("No entries loaded. Open a .ns4b backup above or drag-and-drop .ns4b / .ns4p / .npno / .nsmp4 onto the window.");
                    return;
                }
                for entry in items {
                    if !query.is_empty() && !entry.to_ascii_lowercase().contains(&query) {
                        continue;
                    }
                    if ui
                        .selectable_label(false, entry)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        match self.active_tab {
                            BackupTab::Programs => selected_program = Some(entry.clone()),
                            BackupTab::Pianos => selected_piano = Some(entry.clone()),
                            BackupTab::Samples => selected_sample = Some(entry.clone()),
                        }
                    }
                }
            });

        if let Some(p) = selected_program {
            self.load_program_entry(&p, panel, audio);
        }
        if let Some(p) = selected_piano {
            self.init_before_load(InitTarget::Piano(self.piano_slot_b), panel, audio);
            self.request_piano_entry(&p);
        }
        if let Some(s) = selected_sample {
            self.init_before_load(InitTarget::Synth(self.synth_layer_idx.min(2)), panel, audio);
            self.request_sample_entry(&s);
        }
    }

    /// Renders the docked `.ns4b` archive browser side panel, sliding in and out with
    /// `drawer_open` (egui's animated side panel; it takes no space once fully closed).
    pub fn render_drawer(
        &mut self,
        ctx: &egui::Context,
        panel: &mut Stage4PanelState,
        audio: &AudioHandle,
    ) {
        egui::SidePanel::left("ns4_library_panel")
            .resizable(true)
            .default_width(360.0)
            .min_width(260.0)
            .max_width(580.0)
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .fill(Color32::from_rgb(36, 42, 48))
                    .inner_margin(egui::Margin::same(10)),
            )
            .show_animated(ctx, self.drawer_open, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(RichText::new(crate::branding::LIBRARY_TITLE).color(Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("X")
                            .on_hover_text("Close Library")
                            .clicked()
                        {
                            self.drawer_open = false;
                        }
                        ui.checkbox(&mut self.auto_close_on_load, "Auto-close")
                            .on_hover_text(
                                "Close library panel after selecting a preset or sample",
                            );
                    });
                });
                ui.separator();
                self.render_browser_inline(ui, panel, audio);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_top_bar_and_closed_drawer_run_headless() {
        let mut lib = BackupLibraryState::new();
        let (audio, _rx) = AudioHandle::new();
        let mut panel = Stage4PanelState::default();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| lib.render_top_bar(ui, &mut panel));
            lib.render_drawer(ctx, &mut panel, &audio);
        });
        assert!(!lib.drawer_open);
    }

    #[test]
    fn dropped_backup_opens_settings_library_tab_not_the_drawer() {
        use crate::debug_dialog::{DebugDialogState, DebugTab};
        let meta = b"<backup file_version=\"1\" backup_format_version=\"1\" product_id=\"46\" product_version=\"166\" product_build=\"2934\" manager_version=\"786\" manager_build=\"1234\"/>";
        let zip = super::super::support::build_synthetic_zip_stored(&[
            ("meta.xml", meta),
            ("Piano/Grand/Dummy.npno", b"x"),
        ]);
        let path = std::env::temp_dir().join(format!("ns4_drop_{}.ns4b", std::process::id()));
        std::fs::write(&path, zip).unwrap();

        let mut lib = BackupLibraryState::new();
        let mut dlg = DebugDialogState::default();
        lib.open_backup_in_settings(&path, &mut dlg);
        let _ = std::fs::remove_file(&path);
        assert!(dlg.open && dlg.tab == DebugTab::Library);
        assert!(!lib.drawer_open, "legacy drawer stays closed");
        assert_eq!(lib.pianos.len(), 1);

        // A bad file reports the error and does not open the dialog.
        let mut lib = BackupLibraryState::new();
        let mut dlg = DebugDialogState::default();
        lib.open_backup_in_settings(std::path::Path::new("missing.ns4b"), &mut dlg);
        assert!(!dlg.open && lib.error_message.is_some());
    }

    /// Headless harness: drawer + a central button, frames 50 ms apart.
    struct Rig {
        ctx: egui::Context,
        lib: BackupLibraryState,
        panel: Stage4PanelState,
        audio: AudioHandle,
        time: f64,
        left: f32,
        button: egui::Rect,
        clicks: u32,
    }

    impl Rig {
        fn new() -> Self {
            let mut rig = Self {
                ctx: egui::Context::default(),
                lib: BackupLibraryState::new(),
                panel: Stage4PanelState::default(),
                audio: AudioHandle::new().0,
                time: 1.0,
                left: 0.0,
                button: egui::Rect::NOTHING,
                clicks: 0,
            };
            rig.frame(vec![]);
            rig
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1200.0, 600.0),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (lib, panel, audio) = (&mut self.lib, &mut self.panel, &self.audio);
            let (mut left, mut button, mut clicks) = (self.left, self.button, self.clicks);
            let _ = self.ctx.run(input, |ctx| {
                lib.render_drawer(ctx, panel, audio);
                left = ctx.available_rect().left();
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = ui.button("central control");
                    button = r.rect;
                    clicks += u32::from(r.clicked());
                });
            });
            (self.left, self.button, self.clicks) = (left, button, clicks);
        }

        fn settle(&mut self) {
            for _ in 0..12 {
                self.frame(vec![]);
            }
        }

        fn click(&mut self, at: egui::Pos2) {
            let ev = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![ev(true)]);
            self.frame(vec![ev(false)]);
        }
    }

    #[test]
    fn drawer_slides_in_and_out_and_takes_no_space_when_closed() {
        let mut rig = Rig::new();
        rig.settle();
        assert_eq!(rig.left, 0.0, "closed drawer leaves the panel full width");
        rig.lib.drawer_open = true;
        rig.frame(vec![]);
        let mid = rig.left;
        rig.settle();
        let open = rig.left;
        assert!((260.0..=580.0).contains(&open), "open width {open}");
        assert!(mid < open, "slides in over several frames ({mid} < {open})");
        rig.lib.drawer_open = false;
        rig.frame(vec![]);
        assert!(rig.left < open, "slides out");
        rig.settle();
        assert_eq!(rig.left, 0.0);
    }

    #[test]
    fn clicks_reach_the_panel_beside_the_drawer_and_not_through_it() {
        let mut rig = Rig::new();
        rig.lib.drawer_open = true;
        rig.settle();
        assert!(
            rig.button.left() >= rig.left,
            "button sits right of the drawer"
        );
        rig.click(rig.button.center());
        assert_eq!(rig.clicks, 1, "the panel stays clickable beside the drawer");
        let inside = egui::pos2(rig.left * 0.5, 300.0);
        rig.click(inside);
        assert_eq!(
            rig.clicks, 1,
            "a click inside the drawer never reaches the panel"
        );
        assert!(rig.lib.drawer_open);
    }

    #[test]
    fn l_toggles_the_drawer() {
        let mut rig = Rig::new();
        let l = |pressed| egui::Event::Key {
            key: egui::Key::L,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut toggled = false;
        rig.time += 0.05;
        let input = egui::RawInput {
            time: Some(rig.time),
            events: vec![l(true)],
            ..Default::default()
        };
        let lib = &mut rig.lib;
        let _ = rig.ctx.run(input, |ctx| toggled = lib.handle_keys(ctx));
        assert!(toggled && rig.lib.drawer_open);
    }
}
