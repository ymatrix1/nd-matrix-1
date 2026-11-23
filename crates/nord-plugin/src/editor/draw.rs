//! One editor frame: read-back, top bar, the baked front panel, sink pump.
//!
//! The panel is the same baked HTML/CSS-ported hardware plate the standalone app draws
//! (`render_stage4_panel` with `use_baked_plate`, `nord-gui`). It fits itself to the window
//! (aspect preserved), so the plugin only reserves the top bar and keeps the window at the
//! plate's aspect with its own resize handle. egui's zoom factor is NOT used: `egui_baseview`
//! lays out with the native pixels-per-point, so zoom only changed font rounding. The host's DPI
//! scale multiplies on top through `nih_plug_egui`.

use super::coverage::Coverage;
use super::index::ParamIndex;
use super::notes::NoteSender;
use super::readback::sync_panel_from_params;
use super::sink::{pump, ParamSink};
use super::top_bar::{self, BarClick, BAR_HEIGHT};
use super::write::ParamWriter;
use crate::loader::LoadSender;
use crossbeam_channel::Receiver;
use nih_plug_egui::egui::{
    self, Area, Context, Id, Order, Pos2, Rect, Sense, TopBottomPanel, Vec2,
};
use nih_plug_egui::resizable_window::paint_resize_corner;
use nih_plug_egui::EguiState;
use nord_gui::audio::{AudioCommand, AudioHandle};
use nord_gui::debug_dialog::{show_debug_dialog, DebugDialogState, DebugTab};
use nord_gui::firmware_panel::FirmwarePanelState;
use nord_gui::library::BackupLibraryState;
use nord_gui::os_update_panel::OsUpdatePanel;
use nord_gui::panel::{render_stage4_panel, Stage4PanelState};
use nord_gui::style::CHASSIS_DARK;
use std::sync::Arc;

/// Baked plate size in pixels (`crates/nord-gui/assets/baked/panel_layout.json` `chassis_px`).
/// VERIFIED: matches the layout asset, checked by `plate_constants_match_the_baked_layout`.
pub const PLATE_W: f32 = 2632.0;
pub const PLATE_H: f32 = 650.0;
/// Default editor window size: plate at about 60 % plus the top bar.
pub const DEFAULT_SIZE: (u32, u32) = (1600, 417);
/// Smallest editor width the resize handle accepts (logical points).
pub const MIN_WIDTH: f32 = 640.0;

/// Everything the editor keeps between frames (GUI thread only).
pub struct EditorState {
    pub panel: Stage4PanelState,
    pub handle: AudioHandle,
    pub rx: Receiver<AudioCommand>,
    pub index: Arc<ParamIndex>,
    pub coverage: Coverage,
    pub notes: NoteSender,
    /// Panel commands dropped for lack of a host parameter.
    pub unmapped: u64,
    /// Last top-bar pill clicked.
    pub bar_click: Option<BarClick>,
    /// Where programs, sound banks and firmware options go (`loader.rs`).
    pub loads: LoadSender,
    /// The `.ns4b` browser and the async decoder behind OPEN .NS4B (shared with the standalone).
    pub library: BackupLibraryState,
    /// The Firmware & DSP tab state; its changes reach the engine as `LoadJob::Firmware`.
    pub firmware: FirmwarePanelState,
    /// The OS update tab; created when the dialog first opens (it parses the saved file).
    pub os_update: Option<OsUpdatePanel>,
    /// The Studio Settings dialog (DEBUG / HELP / OPEN .NS4B), an embedded window in the editor.
    pub debug_dialog: DebugDialogState,
}

impl EditorState {
    pub fn new(index: Arc<ParamIndex>, notes: NoteSender, loads: LoadSender) -> Self {
        let (handle, rx) = AudioHandle::new();
        let coverage = super::coverage::classify(&index);
        Self {
            panel: Stage4PanelState {
                use_baked_plate: true,
                ..Default::default()
            },
            handle,
            rx,
            index,
            coverage,
            notes,
            unmapped: 0,
            bar_click: None,
            loads,
            library: plugin_library(),
            firmware: FirmwarePanelState::new(Default::default()),
            os_update: None,
            debug_dialog: DebugDialogState::default(),
        }
    }

    /// What a top-bar pill does, like the standalone bar: OPEN .NS4B opens the Library tab,
    /// DEBUG toggles the dialog on its last tab, ? HELP opens the Setup Guide.
    pub fn open_for(&mut self, click: BarClick) {
        self.bar_click = Some(click);
        match click {
            BarClick::OpenNs4b => self.debug_dialog.open_tab(DebugTab::Library),
            BarClick::Help => self.debug_dialog.open_tab(DebugTab::Guide),
            BarClick::Debug if self.debug_dialog.open => self.debug_dialog.open = false,
            BarClick::Debug => {
                let tab = self.debug_dialog.tab;
                self.debug_dialog.open_tab(tab);
            }
        }
    }
}

/// A library that starts empty: the standalone pre-fills the path with a `.ns4b` found in the
/// working directory, which in a DAW is an arbitrary folder.
fn plugin_library() -> BackupLibraryState {
    let mut library = BackupLibraryState::new();
    library.path_input.clear();
    library
}

/// Loads the dialog / library / firmware panel sent, and the Nord OS update tab.
fn show_dialogs(ctx: &Context, st: &mut EditorState) {
    st.library.poll_results(&mut st.panel, &st.handle);
    if st.debug_dialog.open {
        let os_update = st.os_update.get_or_insert_with(|| {
            let env = std::env::var(crate::wave_tables::WAVE_TABLES_ENV).ok();
            OsUpdatePanel::new(
                nord_core::user_config::default_config_path(),
                env.as_deref(),
            )
        });
        show_debug_dialog(
            ctx,
            &mut st.debug_dialog,
            &mut st.panel,
            &mut st.library,
            &mut st.firmware,
            os_update,
            &st.handle,
        );
        if !st.debug_dialog.open {
            // A newly chosen OS update file is installed once, off-thread (`wave_tables.rs`).
            crate::wave_tables::install_saved_in_background();
        }
    }
    st.library.handle_keys(ctx);
    st.library.render_drawer(ctx, &mut st.panel, &st.handle);
}

/// Window size for a plate `width` wide: the plate aspect plus the top bar.
pub fn size_for_width(width: f32) -> (u32, u32) {
    let w = width.max(MIN_WIDTH);
    (
        w.round() as u32,
        (w * PLATE_H / PLATE_W + BAR_HEIGHT).round() as u32,
    )
}

/// Window size the handle asks for when the pointer is at `pointer`: the aspect is locked to the
/// plate's, so the window never letterboxes.
pub fn resize_target(pointer: Pos2, origin: Pos2, handle: f32) -> (u32, u32) {
    size_for_width(pointer.x - origin.x + 0.5 * handle)
}

/// Drag handle in the bottom-right corner, in a foreground layer above the panel.
fn resize_handle(ctx: &Context, egui_state: &EguiState, avail: Rect) {
    let handle = 18.0_f32;
    Area::new(Id::new("nd-matrix-1-resize"))
        .order(Order::Foreground)
        .fixed_pos(avail.max - Vec2::splat(handle))
        .constrain(false)
        .show(ctx, |ui| {
            let (_rect, resp) = ui.allocate_exact_size(Vec2::splat(handle), Sense::drag());
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    egui_state.set_requested_size(resize_target(p, avail.min, handle));
                }
            }
            paint_resize_corner(ui, &resp);
        });
}

/// Draw one frame of the editor and forward the panel's edits to `writer`.
pub fn draw_editor<W: ParamWriter>(
    ctx: &Context,
    egui_state: &EguiState,
    st: &mut EditorState,
    writer: W,
) {
    sync_panel_from_params(&st.index, &mut st.panel);

    // The shared panel would otherwise drive egui's zoom itself; the editor shows the whole plate.
    st.panel.view.host_scaled = true;
    st.panel.view.focus = None;
    st.panel.view.panorama = None;
    if (ctx.zoom_factor() - 1.0).abs() > 1e-3 {
        ctx.set_zoom_factor(1.0);
    }

    let bar_frame = egui::Frame::NONE.fill(CHASSIS_DARK);
    TopBottomPanel::top("nd_matrix_bar")
        .exact_height(BAR_HEIGHT)
        .show_separator_line(false)
        .frame(bar_frame)
        .show(ctx, |ui| {
            if let Some(click) = top_bar::show(ui, st.debug_dialog.open) {
                st.open_for(click);
            }
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(CHASSIS_DARK))
        .show(ctx, |ui| {
            render_stage4_panel(ui, &mut st.panel, &st.handle);
        });
    show_dialogs(ctx, st);
    resize_handle(ctx, egui_state, ctx.screen_rect());

    let mut sink = ParamSink::new(writer, &st.notes).with_loads(&st.loads);
    pump(&st.rx, &mut sink);
    st.unmapped += sink.unmapped;
    if sink.dropped_loads > 0 {
        st.library.error_message = Some(
            "Load not applied: the plugin is not processing audio. Start playback and retry."
                .into(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::notes::note_queue;
    use crate::editor::sink::test_support::Recorder;
    use crate::params::Stage4Params;
    use nord_gui::baked::BakedPanelAssets;

    fn state() -> (
        Stage4Params,
        EditorState,
        crate::editor::notes::NoteReceiver,
    ) {
        let params = Stage4Params::default();
        let index = Arc::new(ParamIndex::new(&params));
        let (tx, rx) = note_queue(64);
        let (load_tx, load_rx) = crate::loader::load_queue(8);
        std::mem::forget(load_rx); // keep the queue connected; these tests do not read it
        (params, EditorState::new(index, tx, load_tx), rx)
    }

    fn input(size: Vec2, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        }
    }

    fn default_size_vec() -> Vec2 {
        Vec2::new(DEFAULT_SIZE.0 as f32, DEFAULT_SIZE.1 as f32)
    }

    #[test]
    fn plate_constants_match_the_baked_layout() {
        let ctx = Context::default();
        let mut got = (0, 0);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let l = BakedPanelAssets::get_or_load(ctx).layout;
            got = (l.chassis_px.width, l.chassis_px.height);
        });
        assert_eq!(got, (PLATE_W as u32, PLATE_H as u32));
    }

    #[test]
    fn the_editor_draws_the_baked_plate() {
        let (_params, st, _rx) = state();
        assert!(st.panel.use_baked_plate);
    }

    #[test]
    fn default_size_has_the_plate_aspect_plus_the_bar() {
        let (w, h) = DEFAULT_SIZE;
        let plate_h = h as f32 - BAR_HEIGHT;
        let a = w as f32 / plate_h;
        assert!((a - PLATE_W / PLATE_H).abs() < 0.02, "{a}");
        assert_eq!(size_for_width(w as f32), DEFAULT_SIZE);
    }

    #[test]
    fn size_for_width_keeps_the_aspect_and_a_minimum() {
        let (w, h) = size_for_width(1316.0);
        assert_eq!(w, 1316);
        assert_eq!(h, (1316.0 * PLATE_H / PLATE_W + BAR_HEIGHT).round() as u32);
        assert_eq!(size_for_width(10.0).0, MIN_WIDTH as u32);
    }

    #[test]
    fn resize_target_follows_the_pointer_with_the_aspect_locked() {
        let t = resize_target(Pos2::new(1000.0, 400.0), Pos2::ZERO, 18.0);
        assert_eq!(t, size_for_width(1009.0));
    }

    #[test]
    fn a_headless_frame_draws_the_panel() {
        let (_params, mut st, _rx) = state();
        let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        let ctx = Context::default();
        let out = ctx.run(input(default_size_vec(), vec![]), |ctx| {
            draw_editor(ctx, &egui_state, &mut st, Recorder::default());
        });
        assert!(out.shapes.len() > 100, "only {} shapes", out.shapes.len());
    }

    /// Regression: the shared panel used to set its own zoom every frame (flip-flop with the
    /// editor's), so the drawn size alternated frame to frame (flicker in FL Studio).
    #[test]
    fn the_drawn_size_is_stable_across_frames() {
        let (_params, mut st, _rx) = state();
        let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        let ctx = Context::default();
        let size = default_size_vec();
        let mut bounds = vec![];
        for _ in 0..6 {
            let out = ctx.run(input(size, vec![]), |ctx| {
                draw_editor(ctx, &egui_state, &mut st, Recorder::default());
            });
            let r = out.shapes.iter().fold(Rect::NOTHING, |r, s| {
                r.union(s.shape.visual_bounding_rect().intersect(s.clip_rect))
            });
            bounds.push(r);
            assert!((ctx.zoom_factor() - 1.0).abs() < 1e-6);
        }
        for r in &bounds[2..] {
            assert!(r.max.x <= size.x + 1.0 && r.max.y <= size.y + 1.0, "{r:?}");
            assert_eq!(*r, bounds[2]);
        }
    }

    /// The plate fills the window width (no letterbox) at the default size, below the bar.
    #[test]
    fn the_plate_fills_the_default_window() {
        let (_params, mut st, _rx) = state();
        let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        let ctx = Context::default();
        for _ in 0..3 {
            let _ = ctx.run(input(default_size_vec(), vec![]), |ctx| {
                draw_editor(ctx, &egui_state, &mut st, Recorder::default());
            });
        }
        let plate: Rect = ctx
            .data(|d| d.get_temp(Id::new("ns4_baked_plate_rect")))
            .expect("plate rect");
        assert!(
            (plate.width() - DEFAULT_SIZE.0 as f32).abs() < 2.0,
            "{plate:?}"
        );
        assert!(plate.top() >= BAR_HEIGHT - 1.0, "{plate:?}");
    }

    fn frame_with(
        ctx: &Context,
        egui_state: &EguiState,
        st: &mut EditorState,
        size: Vec2,
        events: Vec<egui::Event>,
    ) {
        let _ = ctx.run(input(size, events), |ctx| {
            draw_editor(ctx, egui_state, st, Recorder::default());
        });
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    /// Press the left button at `pos` (window points) for a few frames, then release.
    fn click_at(
        ctx: &Context,
        egui_state: &EguiState,
        st: &mut EditorState,
        size: Vec2,
        pos: Pos2,
    ) {
        for _ in 0..3 {
            frame_with(ctx, egui_state, st, size, vec![]);
        }
        let moved = vec![egui::Event::PointerMoved(pos)];
        frame_with(ctx, egui_state, st, size, moved);
        frame_with(ctx, egui_state, st, size, vec![button(pos, true)]);
        frame_with(ctx, egui_state, st, size, vec![]);
        frame_with(ctx, egui_state, st, size, vec![button(pos, false)]);
        frame_with(ctx, egui_state, st, size, vec![]);
    }

    /// Regression: dragging the bottom-right corner must request a (aspect-locked) resize.
    #[test]
    fn dragging_the_bottom_right_corner_requests_a_resize() {
        let (_params, mut st, _rx) = state();
        let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        let ctx = Context::default();
        let size = default_size_vec();
        let corner = Pos2::new(size.x - 4.0, size.y - 4.0);
        let smaller = corner - Vec2::new(300.0, 80.0);
        frame_with(&ctx, &egui_state, &mut st, size, vec![]);
        let at_corner = vec![egui::Event::PointerMoved(corner)];
        frame_with(&ctx, &egui_state, &mut st, size, at_corner);
        frame_with(&ctx, &egui_state, &mut st, size, vec![button(corner, true)]);
        for _ in 0..2 {
            let moved = vec![egui::Event::PointerMoved(smaller)];
            frame_with(&ctx, &egui_state, &mut st, size, moved);
        }
        let dbg = format!("{egui_state:?}");
        assert!(
            dbg.contains("requested_size: AtomicCell { value: Some("),
            "{dbg}"
        );
    }

    /// Point on the white key `midi` of the baked keybed, in window points (lower part of the
    /// key), after the layout settled.
    fn white_key_point(ctx: &Context, midi: u8) -> Pos2 {
        let plate: Rect = ctx
            .data(|d| d.get_temp(Id::new("ns4_baked_plate_rect")))
            .expect("plate rect");
        let mut p = Pos2::ZERO;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let l = BakedPanelAssets::get_or_load(ctx).layout;
            let k = l
                .keys
                .iter()
                .find(|k| k.midi == midi && !k.black)
                .expect("key");
            p = Pos2::new(
                plate.left() + (k.x + k.w * 0.5) * plate.width(),
                plate.top() + (k.y + k.h * 0.85) * plate.height(),
            );
        });
        p
    }

    /// Regression: clicking the on-screen keybed must play a note (the user reported keys and UI
    /// doing nothing in the plugin: the sink dropped `AudioCommand::Keyboard`).
    #[test]
    fn clicking_a_keybed_key_plays_that_note() {
        let (_params, mut st, notes) = state();
        let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        let ctx = Context::default();
        let size = default_size_vec();
        for _ in 0..3 {
            frame_with(&ctx, &egui_state, &mut st, size, vec![]);
        }
        let pos = white_key_point(&ctx, 60);
        click_at(&ctx, &egui_state, &mut st, size, pos);
        let mut played = vec![];
        while let Some(ev) = notes.pop() {
            played.push(ev);
        }
        assert!(
            played
                .iter()
                .any(|e| matches!(e, nord_core::midi::MidiEvent::NoteOn { note: 60, .. })),
            "{played:?}"
        );
    }

    #[test]
    fn clicking_a_top_bar_pill_is_recorded() {
        let (_params, mut st, _rx) = state();
        let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        let ctx = Context::default();
        let size = default_size_vec();
        // First pill sits at the left of the bar.
        let pos = Pos2::new(30.0, BAR_HEIGHT * 0.5);
        click_at(&ctx, &egui_state, &mut st, size, pos);
        assert_eq!(st.bar_click, Some(BarClick::OpenNs4b));
    }

    #[test]
    fn a_panel_edit_in_a_frame_reaches_the_writer_and_the_note_queue() {
        let (_params, mut st, notes) = state();
        st.handle.send_command(AudioCommand::SetMasterVolume(0.33));
        st.handle.send_midi(nord_core::midi::MidiEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 99,
        });
        let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        let ctx = Context::default();
        let mut rec = Recorder::default();
        // Re-borrow the recorder through a wrapper so the test can inspect it afterwards.
        struct Shared<'a>(&'a mut Recorder);
        impl ParamWriter for Shared<'_> {
            fn write(&mut self, w: &crate::editor::write::ParamWrite) -> bool {
                self.0.write(w)
            }
        }
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            draw_editor(ctx, &egui_state, &mut st, Shared(&mut rec));
        });
        assert_eq!(rec.writes.len(), 1);
        assert_eq!(rec.writes[0].family, "master_level");
        assert!(notes.pop().is_some());
    }
}
