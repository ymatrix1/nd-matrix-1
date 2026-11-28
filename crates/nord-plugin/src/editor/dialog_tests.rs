//! The top-bar pills open the Studio Settings dialog inside the editor window, and what its tabs
//! load reaches the engine through the load queue (D218).

use super::draw::{draw_editor, EditorState, DEFAULT_SIZE};
use super::index::ParamIndex;
use super::notes::note_queue;
use super::sink::test_support::Recorder;
use super::top_bar::BarClick;
use crate::loader::{load_queue, LoadJob, LoadReceiver};
use crate::params::Stage4Params;
use nih_plug_egui::{egui, EguiState};
use nord_gui::debug_dialog::DebugTab;
use std::sync::Arc;

fn editor() -> (EditorState, LoadReceiver) {
    let params = Arc::new(Stage4Params::default());
    let index = Arc::new(ParamIndex::new(params.as_ref()).with_program_hold(params));
    let (tx, _notes) = note_queue(8);
    let (load_tx, load_rx) = load_queue(8);
    (
        EditorState::with_library(index, tx, load_tx, Default::default(), None),
        load_rx,
    )
}

fn frame(ctx: &egui::Context, st: &mut EditorState) -> egui::FullOutput {
    let egui_state = EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
    let size = egui::vec2(DEFAULT_SIZE.0 as f32, DEFAULT_SIZE.1 as f32);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..Default::default()
    };
    ctx.run(input, |ctx| {
        draw_editor(ctx, &egui_state, st, Recorder::default())
    })
}

#[test]
fn every_pill_opens_the_tab_the_standalone_bar_opens() {
    let (mut st, _rx) = editor();
    assert!(!st.debug_dialog.open);
    st.open_for(BarClick::OpenNs4b);
    assert!(st.debug_dialog.open && st.debug_dialog.tab == DebugTab::Library);
    st.open_for(BarClick::Help);
    assert!(st.debug_dialog.open && st.debug_dialog.tab == DebugTab::Guide);
    // DEBUG toggles: it closes an open dialog, and reopens it on the tab last shown.
    st.open_for(BarClick::Debug);
    assert!(!st.debug_dialog.open);
    st.open_for(BarClick::Debug);
    assert!(st.debug_dialog.open && st.debug_dialog.tab == DebugTab::Guide);
}

#[test]
fn the_dialog_is_an_embedded_window_that_fits_inside_the_editor() {
    let (mut st, _rx) = editor();
    let ctx = egui::Context::default();
    let closed = frame(&ctx, &mut st).shapes.len();
    for tab in DebugTab::ALL {
        st.debug_dialog.open_tab(tab);
        // Two frames: the window sizes itself on the first.
        let _ = frame(&ctx, &mut st);
        let shapes = frame(&ctx, &mut st).shapes.len();
        assert!(shapes > closed, "{tab:?} drew nothing");
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("ns4_shadcn_debug_embedded")))
            .unwrap_or_else(|| panic!("{tab:?}: no dialog window"));
        let screen = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(DEFAULT_SIZE.0 as f32, DEFAULT_SIZE.1 as f32),
        );
        assert!(
            screen.contains_rect(rect),
            "{tab:?}: {rect:?} sticks out of {screen:?}"
        );
    }
}

#[test]
fn closing_the_dialog_removes_it() {
    let (mut st, _rx) = editor();
    let ctx = egui::Context::default();
    let closed = frame(&ctx, &mut st).shapes.len();
    st.open_for(BarClick::Help);
    let _ = frame(&ctx, &mut st);
    st.open_for(BarClick::Debug);
    assert_eq!(frame(&ctx, &mut st).shapes.len(), closed);
}

#[test]
fn a_firmware_option_change_is_queued_for_the_engine() {
    let (mut st, rx) = editor();
    let ctx = egui::Context::default();
    st.firmware.opts.synth_wave_bank = true;
    st.firmware.push(&st.handle);
    let _ = frame(&ctx, &mut st);
    match rx.pop() {
        Some(LoadJob::Firmware(o)) => assert!(o.synth_wave_bank),
        other => panic!("expected a firmware job, got {other:?}"),
    }
    assert!(rx.pop().is_none());
}

#[test]
fn the_library_starts_empty_instead_of_guessing_a_backup_in_the_hosts_folder() {
    let (st, _rx) = editor();
    assert!(st.library.path_input.is_empty());
    assert!(st.library.archive.is_none());
}
