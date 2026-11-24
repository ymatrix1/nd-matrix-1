//! The text size option (issue #592) in the editor frame: the bar, the plate, the window size and
//! the Studio Settings dialog at 100 % to 200 %. Headless: egui builds the real shapes and the
//! checks read the painted text rectangles (position, size, clip). These are text-rectangle checks,
//! not golden bitmaps: the headless context has no GPU.

use super::draw::{draw_editor, size_for_width, EditorState, DEFAULT_SIZE, PLATE_H, PLATE_W};
use super::index::ParamIndex;
use super::notes::note_queue;
use super::sink::test_support::Recorder;
use crate::loader::load_queue;
use crate::params::Stage4Params;
use nih_plug_egui::egui::{self, Rect, Shape};
use nih_plug_egui::EguiState;
use nord_gui::debug_dialog::DebugTab;
use nord_gui::text_scale::{self, TextScale};
use std::sync::Arc;

/// Puts the thread-local text factor back to 100 % when a test ends.
struct ResetFactor;

impl Drop for ResetFactor {
    fn drop(&mut self) {
        text_scale::apply(
            &egui::Context::default(),
            TextScale::default(),
            &mut Default::default(),
        );
    }
}

fn editor(percent: u32) -> EditorState {
    let params = Arc::new(Stage4Params::default());
    let index = Arc::new(ParamIndex::new(params.as_ref()).with_program_hold(params));
    let (tx, _notes) = note_queue(8);
    let (load_tx, _load_rx) = load_queue(8);
    let mut st = EditorState::new(index, tx, load_tx, Default::default());
    st.text_scale = TextScale::new(percent);
    st
}

fn vec(size: (u32, u32)) -> egui::Vec2 {
    egui::vec2(size.0 as f32, size.1 as f32)
}

/// One editor window as a host runs it: it opens at `size`, and honours a size the editor asks for
/// (the next frame uses it).
struct Host {
    ctx: egui::Context,
    st: EditorState,
    state: Arc<EguiState>,
    size: (u32, u32),
}

impl Host {
    fn open(percent: u32, size: (u32, u32)) -> Self {
        Self {
            ctx: egui::Context::default(),
            st: editor(percent),
            state: EguiState::from_size(size.0, size.1),
            size,
        }
    }

    /// One frame; returns its output and the size the editor asked for during it.
    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Option<(u32, u32)>) {
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec(self.size))),
            modifiers,
            events,
            ..Default::default()
        };
        let (state, st) = (&self.state, &mut self.st);
        let out = self.ctx.run(input, |ctx| {
            draw_editor(ctx, state, st, Recorder::default())
        });
        let asked = self.state.requested_size();
        if let Some(new) = asked {
            self.size = new;
            self.state = EguiState::from_size(new.0, new.1);
        }
        (out, asked)
    }

    /// Frames until the layout settles (the window may be resized once on the way).
    fn settle(&mut self) -> egui::FullOutput {
        let mut out = self.frame(vec![]).0;
        for _ in 0..5 {
            out = self.frame(vec![]).0;
        }
        out
    }

    fn screen(&self) -> Rect {
        Rect::from_min_size(egui::Pos2::ZERO, vec(self.size))
    }
}

struct Label {
    text: String,
    rect: Rect,
    clip: Rect,
}

/// Every painted text with its rectangle (galley rect at its position) and clip rectangle.
fn labels(out: &egui::FullOutput) -> Vec<Label> {
    let mut v = Vec::new();
    for s in &out.shapes {
        if let Shape::Text(t) = &s.shape {
            v.push(Label {
                text: t.galley.text().to_string(),
                rect: t.galley.rect.translate(t.pos.to_vec2()),
                clip: s.clip_rect,
            });
        }
    }
    v
}

/// Labels that belong to the bar (above the plate).
fn bar_labels(out: &egui::FullOutput) -> Vec<Label> {
    let bar_h = text_scale::bar_height();
    labels(out)
        .into_iter()
        .filter(|l| l.rect.center().y < bar_h)
        .collect()
}

fn default_host(percent: u32) -> Host {
    Host::open(percent, DEFAULT_SIZE)
}

fn assert_bar_whole_and_apart(host: &Host, out: &egui::FullOutput, what: &str) {
    let bar_h = text_scale::bar_height();
    let bar = bar_labels(out);
    assert!(bar.len() >= 10, "{what}: only {} bar labels", bar.len());
    for l in &bar {
        assert!(
            host.screen().contains_rect(l.rect),
            "{what}: {:?} sticks out {:?}",
            l.text,
            l.rect
        );
        assert!(
            l.clip.expand(0.5).contains_rect(l.rect),
            "{what}: {:?} is clipped by {:?} ({:?})",
            l.text,
            l.clip,
            l.rect
        );
        assert!(
            l.rect.height() <= bar_h,
            "{what}: {:?} taller than the bar",
            l.text
        );
    }
    for (i, a) in bar.iter().enumerate() {
        for b in &bar[i + 1..] {
            assert!(
                !a.rect.shrink(0.25).intersects(b.rect.shrink(0.25)),
                "{what}: {:?} overlaps {:?}",
                a.text,
                b.text
            );
        }
    }
}

#[test]
fn bar_text_stays_whole_and_apart_at_every_size() {
    let _reset = ResetFactor;
    for percent in [100, 125, 150, 175, 200] {
        let mut host = default_host(percent);
        let out = host.settle();
        assert_bar_whole_and_apart(&host, &out, &format!("{percent} %"));
    }
}

#[test]
fn an_800_wide_host_is_asked_for_a_window_the_bar_fits_in() {
    let _reset = ResetFactor;
    for percent in [150, 200] {
        let mut host = Host::open(percent, (800, 220));
        let (_, asked) = host.frame(vec![]);
        let asked = asked.unwrap_or_else(|| panic!("{percent} %: no resize request"));
        assert!(
            asked.0 as f32 >= text_scale::bar_content_width(),
            "{percent} %: asked for {asked:?}, the bar needs {}",
            text_scale::bar_content_width()
        );
        assert_eq!(
            asked,
            size_for_width(asked.0 as f32),
            "aspect and bar height"
        );
        let out = host.settle();
        assert_bar_whole_and_apart(&host, &out, &format!("800 wide host at {percent} %"));
    }
    // At 100 % an 800 wide window is already wide enough: only the height follows the aspect.
    let mut host = Host::open(100, (800, 417));
    let (_, asked) = host.frame(vec![]);
    assert_eq!(asked, Some(size_for_width(800.0)));
}

#[test]
fn bar_width_formula_covers_the_measured_bar() {
    let _reset = ResetFactor;
    for percent in [100, 150, 200] {
        let mut host = default_host(percent);
        let out = host.settle();
        let right = bar_labels(&out)
            .iter()
            .fold(0.0_f32, |m, l| m.max(l.rect.right()));
        let formula = text_scale::bar_content_width();
        assert!(
            right <= formula && formula - right < 12.0,
            "{percent} %: bar ends at {right}, formula {formula}"
        );
    }
}

/// Text rows are pixel-rounded, hence whole numbers. EMPIRICAL: measured with this test on egui
/// 0.31.1 bundled fonts, 2026-11-24; a font or style change shows up here first.
#[test]
fn pill_text_grows_with_the_chosen_size() {
    let _reset = ResetFactor;
    let mut heights = Vec::new();
    for percent in [100, 150, 200] {
        let mut host = default_host(percent);
        let out = host.settle();
        let debug = bar_labels(&out)
            .into_iter()
            .find(|l| l.text == "DEBUG")
            .unwrap_or_else(|| panic!("{percent} %: no DEBUG pill"));
        heights.push((percent, debug.rect.height()));
    }
    assert_eq!(heights, [(100, 10.0), (150, 16.0), (200, 21.0)]);
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }]
}

#[test]
fn changing_the_size_asks_the_host_for_the_matching_window() {
    let _reset = ResetFactor;
    let mut host = default_host(100);
    let (_, first) = host.frame(vec![]);
    assert_eq!(first, None, "the default window already matches 100 %");
    host.settle();
    let (_, asked) = host.frame(key(egui::Key::Equals, egui::Modifiers::COMMAND));
    assert_eq!(host.st.text_scale.percent(), 125);
    let (w, h) = asked.expect("a resize request");
    assert_eq!((w, h), size_for_width(DEFAULT_SIZE.0 as f32));
    let want_h = (w as f32 * PLATE_H / PLATE_W + text_scale::bar_height()).round() as u32;
    assert_eq!(h, want_h);
    assert!(h > DEFAULT_SIZE.1, "a taller bar needs a taller window");
    // An editor that opens at a saved 150 % is sized on its first frame.
    let mut saved = default_host(150);
    let (_, asked) = saved.frame(vec![]);
    assert_eq!(asked, Some(size_for_width(DEFAULT_SIZE.0 as f32)));
    assert_ne!(asked, Some(DEFAULT_SIZE));
}

#[test]
fn the_window_keeps_the_plate_aspect_below_the_taller_bar() {
    let _reset = ResetFactor;
    for percent in [100u32, 150, 200] {
        let _ = default_host(percent).settle(); // sets the factor of this thread
        let (w, h) = size_for_width(1600.0);
        let bar = text_scale::bar_height();
        assert_eq!(
            h,
            (1600.0 * PLATE_H / PLATE_W + bar).round() as u32,
            "{percent} %"
        );
        assert_eq!(w, 1600);
    }
}

#[test]
fn the_plate_is_magnified_by_the_size_and_starts_at_the_top_left() {
    let _reset = ResetFactor;
    for percent in [100u32, 150, 200] {
        let mut host = default_host(percent);
        host.settle();
        let f = percent as f32 / 100.0;
        let zoom = host.st.panel.baked_zoom;
        assert!((zoom.base - f).abs() < 1e-4);
        assert!((zoom.zoom - f).abs() < 1e-3, "{percent} %: {zoom:?}");
        let plate: Rect = host
            .ctx
            .data(|d| d.get_temp(egui::Id::new("ns4_baked_plate_rect")))
            .expect("plate rect");
        // The visible part fills the window width; the whole plate is f times wider.
        assert!(
            (plate.width() - host.size.0 as f32).abs() < 2.0,
            "{plate:?}"
        );
        assert!(plate.left().abs() < 1.0, "{percent} %: starts at {plate:?}");
    }
}

#[test]
fn the_dialog_scales_and_fits_the_editor_at_every_size() {
    let _reset = ResetFactor;
    let title = "Studio Settings & Diagnostics";
    let mut title_heights = Vec::new();
    for percent in [100, 150, 200] {
        let mut host = default_host(percent);
        host.settle();
        for tab in DebugTab::ALL {
            host.st.debug_dialog.open_tab(tab);
            let mut out = host.frame(vec![]).0;
            for _ in 0..3 {
                out = host.frame(vec![]).0;
            }
            let rect = host
                .ctx
                .memory(|m| m.area_rect(egui::Id::new("ns4_shadcn_debug_embedded")))
                .unwrap_or_else(|| panic!("{percent} % {tab:?}: no dialog window"));
            assert!(
                host.screen().contains_rect(rect),
                "{percent} % {tab:?}: {rect:?} sticks out of {:?}",
                host.screen()
            );
            assert!(
                rect.width() > 200.0 && rect.height() > 100.0,
                "{tab:?}: {rect:?}"
            );
            if tab == DebugTab::Guide {
                let t = labels(&out)
                    .into_iter()
                    .find(|l| l.text == title)
                    .unwrap_or_else(|| panic!("{percent} %: no dialog title"));
                assert!(
                    host.screen().contains_rect(t.rect),
                    "{percent} %: {:?}",
                    t.rect
                );
                title_heights.push(t.rect.height());
            }
        }
    }
    // The dialog text itself grows with the size (not only the bar).
    assert!(
        title_heights[2] > title_heights[1] && title_heights[1] > title_heights[0],
        "{title_heights:?}"
    );
}

#[test]
fn ctrl_plus_minus_zero_change_the_size_and_save_it() {
    let _reset = ResetFactor;
    let dir = std::env::temp_dir().join(format!("ns4-ts-editor-{}", std::process::id()));
    let cfg = dir.join("ns4").join("config.json");
    let _ = std::fs::remove_dir_all(&dir);
    let mut host = default_host(100);
    host.st.config_path = Some(cfg.clone());
    let ctrl = egui::Modifiers::COMMAND;
    let _ = host.frame(vec![]);
    let _ = host.frame(key(egui::Key::Equals, ctrl));
    let _ = host.frame(key(egui::Key::Plus, ctrl));
    assert_eq!(host.st.text_scale.percent(), 150);
    assert_eq!(text_scale::load(Some(&cfg)).percent(), 150, "saved");
    let _ = host.frame(key(egui::Key::Minus, ctrl));
    assert_eq!(host.st.text_scale.percent(), 125);
    let _ = host.frame(key(egui::Key::Num0, ctrl));
    assert_eq!(host.st.text_scale.percent(), 100);
    assert_eq!(text_scale::load(Some(&cfg)).percent(), 100);
    // Without Ctrl the same keys do nothing to the text size.
    let _ = host.frame(key(egui::Key::Plus, egui::Modifiers::NONE));
    assert_eq!(host.st.text_scale.percent(), 100);
    let _ = std::fs::remove_dir_all(&dir);
}
