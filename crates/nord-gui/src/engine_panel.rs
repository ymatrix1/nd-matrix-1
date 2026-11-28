//! "Engine" selector of the Settings & Diagnostics dialog (OS & Wave Tables tab, bean `ns4-lle7`,
//! D246): the built-in engine (HLE, "Normal", default) or the opt-in low-level engine (LLE). Shared
//! by `nd-matrix-1` and the DAW plugin through `os_update_panel`. It edits the **saved** choice and
//! shows what runs this session: `--engine` or `NS4_ENGINE` override the saved choice for one run
//! (read once at start, like `engine_select::startup`). There is no live switch: a change applies at
//! the next start. If LLE is chosen but cannot run (no update file, or no LLE engine in this build)
//! the built-in engine keeps running and the panel says why; it never switches silently.

use nord_core::engine_select::{self, EngineChoice, EngineKind, LleAvailability, LleBackend};
use nord_core::engine_status::{self, EngineStatus};
use nord_core::precision::{self, PrecisionMode};
use std::path::{Path, PathBuf};

/// Shown under the selector. ASSUMED wording: "may not run in real time" is the D246/D249 verdict
/// (exact LLE is x3.5 over the pass budget, `docs/research/sharc-lle-realtime.md`).
const EXPLAIN: &str = "The built-in engine (HLE) is the default and plays in real time on every machine. The low-level engine (LLE) runs your own Nord OS update file instruction by instruction: it is exact, but may not run in real time on every machine. It is opt-in and is never used unless you choose it.";

/// Plain name of an engine for the status lines.
fn name(k: EngineKind) -> &'static str {
    match k {
        EngineKind::Hle => "Built-in (HLE, default)",
        EngineKind::Lle => "Low-level (LLE)",
    }
}

/// State of the engine selector.
pub struct EnginePanel {
    config: Option<PathBuf>,
    /// The saved value as text (unknown values stay visible in the notes).
    saved: Option<String>,
    /// The saved choice as a radio value (HLE when absent or unknown).
    requested: EngineKind,
    /// `--engine` or `NS4_ENGINE` of this run, as typed, with its name; flag first.
    run_override: Option<(String, &'static str)>,
    precision: PrecisionMode,
    message: Option<String>,
}

/// The value after `--engine` (as typed) or the environment value, flag first.
fn run_override(args: &[String], env: Option<String>) -> Option<(String, &'static str)> {
    let flag = args
        .iter()
        .position(|a| a == "--engine")
        .and_then(|i| args.get(i + 1))
        .cloned();
    match (flag, env) {
        (Some(v), _) => Some((v, "--engine")),
        (None, Some(v)) => Some((v, engine_select::ENGINE_ENV)),
        _ => None,
    }
}

impl EnginePanel {
    /// Reads the saved choice from `config` and the flag and environment of this process; an
    /// unreadable file or unknown value keeps HLE and says so.
    pub fn new(config: Option<PathBuf>) -> Self {
        let args: Vec<String> = std::env::args().collect();
        let env = std::env::var(engine_select::ENGINE_ENV).ok();
        Self::new_with(config, &args, env)
    }

    /// [`Self::new`] with the command line and environment value given (no process state).
    pub fn new_with(config: Option<PathBuf>, args: &[String], env: Option<String>) -> Self {
        let mut message = None;
        let mut precision = PrecisionMode::Exact;
        let mut saved = None;
        match config.as_deref().map(nord_core::user_config::load) {
            Some(Ok(c)) => {
                let (p, notice) = precision::resolve(&[], None, c.precision.as_deref());
                precision = p;
                message = notice;
                saved = c.engine;
            }
            Some(Err(e)) => message = Some(format!("saved settings not read ({e}).")),
            None => {}
        }
        let requested = EngineKind::parse(saved.as_deref().unwrap_or("")).unwrap_or_default();
        Self {
            config,
            saved,
            requested,
            run_override: run_override(args, env),
            precision,
            message,
        }
    }

    /// Everything the panel shows for the effective update file.
    pub fn status(&self, os_update: Option<&Path>) -> EngineStatus {
        let over = self.run_override.as_ref().map(|(v, _)| v.as_str());
        engine_status::status_with(over, self.saved.as_deref(), os_update, LleBackend::probe)
    }

    /// The engine that runs for the saved choice and any override, given the update file.
    pub fn choice(&self, os_update: Option<&Path>) -> EngineChoice {
        let mut c = self.status(os_update).choice;
        c.precision = self.precision;
        c
    }
    /// The precision selector is live only while the LLE engine is the requested one.
    pub fn precision_enabled(&self) -> bool {
        self.requested == EngineKind::Lle
    }

    /// Chooses the SHARC float precision and saves it (plumbed to the LLE backend seam).
    pub fn select_precision(&mut self, mode: PrecisionMode) {
        self.precision = mode;
        self.message = Some(self.save_with("precision", |c| precision::save_choice(c, mode)));
    }

    /// Chooses `kind` and saves it; the message says what happened in plain words.
    pub fn select(&mut self, kind: EngineKind) {
        self.requested = kind;
        self.saved = Some(kind.as_str().into());
        self.message = Some(self.save_with("engine", |c| engine_select::save_choice(c, kind)));
    }

    /// Runs `save` on the config path and words the outcome for the user.
    fn save_with(&self, what: &str, save: impl FnOnce(&Path) -> Result<(), String>) -> String {
        match &self.config {
            None => {
                "This system has no place to save settings; the choice lasts until you quit.".into()
            }
            Some(c) => match save(c) {
                Ok(()) => format!("Saved. Restart the app to apply the {what} choice."),
                Err(e) => format!("Not saved: {e}"),
            },
        }
    }

    /// Draws the selector, the status lines and the fallback notice.
    pub fn show(&mut self, ui: &mut egui::Ui, os_update: Option<&Path>) {
        ui.strong("Engine (experimental)");
        ui.label(EXPLAIN);
        let mut pick = self.requested;
        ui.horizontal(|ui| {
            ui.label("Saved choice:");
            ui.radio_value(&mut pick, EngineKind::Hle, "Built-in (HLE)");
            ui.radio_value(&mut pick, EngineKind::Lle, "Low-level (LLE)");
        });
        if pick != self.requested {
            self.select(pick);
        }
        let mut prec = self.precision;
        let enabled = self.precision_enabled();
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Precision:");
                ui.radio_value(&mut prec, PrecisionMode::Exact, "Exact (bit-exact, 40-bit)");
                ui.radio_value(&mut prec, PrecisionMode::Fast, "Fast (f32, real-time)");
            });
        });
        if !enabled {
            ui.label("Precision applies only to the low-level engine (LLE); select it above.");
        }
        if enabled && prec != self.precision {
            self.select_precision(prec);
        }
        self.show_status(ui, &self.status(os_update));
        ui.label("There is no live switch: a change applies the next time the app starts.");
        if let Some(m) = &self.message {
            ui.label(m);
        }
    }

    /// The read-only lines: what runs now, what the build can do, and why they differ.
    fn show_status(&self, ui: &mut egui::Ui, st: &EngineStatus) {
        ui.label(format!(
            "Running this session: {}",
            name(st.choice.effective)
        ));
        if let Some((v, src)) = &self.run_override {
            ui.label(format!(
                "{src} = \"{v}\" overrides the saved choice for this run."
            ));
        }
        ui.label(match &st.backend {
            LleAvailability::Available => "Low-level engine in this build: available".to_string(),
            LleAvailability::Unavailable(r) => {
                format!("Low-level engine in this build: not available ({r})")
            }
        });
        ui.label(if st.os_update_present {
            "Nord OS update file: set"
        } else {
            "Nord OS update file: not set (the low-level engine needs your own copy)"
        });
        if let Some(n) = &st.choice.notice {
            ui.label(n);
        }
        for n in &st.notes {
            ui.label(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("ns4-gui-engine-{name}-{}", std::process::id()))
            .join("config.json")
    }

    /// A panel for a run with no flag and no `NS4_ENGINE`, whatever the test process has.
    fn panel(config: Option<PathBuf>) -> EnginePanel {
        EnginePanel::new_with(config, &[], None)
    }

    fn texts(out: &egui::FullOutput) -> String {
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::epaint::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn default_is_hle_without_a_notice() {
        let p = panel(Some(config("default")));
        let c = p.choice(None);
        assert_eq!((c.effective, c.notice), (EngineKind::Hle, None));
    }

    #[test]
    fn choosing_lle_saves_it_and_falls_back_with_a_prompt_then_the_reason() {
        let c = config("lle");
        let mut p = panel(Some(c.clone()));
        p.select(EngineKind::Lle);
        assert!(p.message.as_deref().unwrap().contains("Saved"));
        let again = panel(Some(c));
        assert_eq!(again.requested, EngineKind::Lle);
        let no_file = again.choice(None);
        assert_eq!(no_file.effective, EngineKind::Hle);
        assert!(no_file.notice.unwrap().contains("Nord OS update file"));
        let stub = again.choice(Some(Path::new("os.zip")));
        assert!(stub.notice.unwrap().contains("not implemented"));
    }

    #[test]
    fn precision_is_saved_enabled_only_for_lle_and_a_bad_value_is_reported() {
        let c = config("precision");
        let mut p = panel(Some(c.clone()));
        assert!(!p.precision_enabled());
        assert_eq!(p.choice(None).precision, PrecisionMode::Exact);
        p.select(EngineKind::Lle);
        assert!(p.precision_enabled());
        p.select_precision(PrecisionMode::Fast);
        let again = panel(Some(c.clone()));
        assert_eq!(again.choice(None).precision, PrecisionMode::Fast);
        // the engine key survived the precision save
        assert_eq!(again.requested, EngineKind::Lle);
        nord_core::user_config::update(&c, |u| u.precision = Some("half".into())).unwrap();
        let bad = panel(Some(c));
        assert_eq!(bad.choice(None).precision, PrecisionMode::Exact);
        assert!(bad.message.unwrap().contains("half"));
    }

    #[test]
    fn a_corrupt_or_unsaveable_config_is_reported_not_fatal() {
        let c = config("bad");
        std::fs::create_dir_all(c.parent().unwrap()).unwrap();
        std::fs::write(&c, "{ nope").unwrap();
        let p = panel(Some(c));
        assert_eq!(p.requested, EngineKind::Hle);
        assert!(p.message.unwrap().contains("not read"));
        let mut none = panel(None);
        none.select(EngineKind::Lle);
        assert!(none.message.unwrap().contains("no place to save"));
    }

    #[test]
    fn a_flag_beats_the_environment_beats_the_saved_choice() {
        let c = config("precedence");
        nord_core::engine_select::save_choice(&c, EngineKind::Lle).unwrap();
        let file = Some(std::env::temp_dir());
        let saved = panel(Some(c.clone()));
        assert_eq!(saved.status(file.as_deref()).env_override, None);
        let env = EnginePanel::new_with(Some(c.clone()), &[], Some("hle".into()));
        let st = env.status(file.as_deref());
        assert_eq!(
            (st.saved, st.env_override),
            (EngineKind::Lle, Some(EngineKind::Hle))
        );
        assert_eq!(st.choice.effective, EngineKind::Hle);
        let args = vec!["x".into(), "--engine".into(), "lle".into()];
        let flag = EnginePanel::new_with(Some(c), &args, Some("hle".into()));
        assert_eq!(flag.run_override, Some(("lle".into(), "--engine")));
        assert_eq!(
            flag.status(file.as_deref()).choice.requested,
            EngineKind::Lle
        );
    }

    #[test]
    fn an_unknown_saved_value_reads_as_hle_and_is_named() {
        let c = config("unknown");
        nord_core::user_config::update(&c, |u| u.engine = Some("turbo".into())).unwrap();
        let p = panel(Some(c));
        assert_eq!(p.requested, EngineKind::Hle);
        let st = p.status(None);
        assert_eq!(st.choice.effective, EngineKind::Hle);
        assert!(st.notes[0].contains("turbo"));
    }

    #[test]
    fn saving_the_engine_keeps_text_size_and_the_update_path() {
        let c = config("keep");
        nord_core::user_config::update(&c, |u| {
            u.text_scale_percent = Some(150);
            u.os_update_path = Some("/os.zip".into());
        })
        .unwrap();
        let mut p = panel(Some(c.clone()));
        p.select(EngineKind::Lle);
        p.select_precision(PrecisionMode::Fast);
        let u = nord_core::user_config::load(&c).unwrap();
        assert_eq!(
            (
                u.text_scale_percent,
                u.os_update_path.as_deref(),
                u.engine.as_deref()
            ),
            (Some(150), Some("/os.zip"), Some("lle"))
        );
        p.select(EngineKind::Hle);
        let u = nord_core::user_config::load(&c).unwrap();
        assert_eq!(
            (u.text_scale_percent, u.precision.as_deref()),
            (Some(150), Some("fast"))
        );
    }

    #[test]
    fn the_panel_explains_availability_and_the_fallback_reason() {
        let mut p = panel(Some(config("text")));
        p.select(EngineKind::Lle);
        let file = std::env::temp_dir();
        let ctx = egui::Context::default();
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| p.show(ui, Some(&file)));
        });
        let all = texts(&out);
        for want in [
            "default",
            "may not run in real time",
            "Running this session: Built-in",
            "not available (the low-level engine (LLE) is not implemented",
            "Nord OS update file: set",
            "Using the built-in engine",
            "next time the app starts",
        ] {
            assert!(all.contains(want), "missing {want:?} in:\n{all}");
        }
        // LLE is saved but no update file is set: the prompt is the notice
        let n = p.choice(None).notice.unwrap();
        assert!(n.contains("Nord OS update file"));
    }

    #[test]
    fn the_settings_dialog_shows_the_engine_selector_on_the_os_update_tab() {
        use crate::debug_dialog::{show_debug_dialog, DebugDialogState, DebugTab};
        use crate::{audio::AudioHandle, firmware_panel::FirmwarePanelState as Fw};
        use crate::{library::BackupLibraryState, os_update_panel::OsUpdatePanel};
        let ctx = egui::Context::default();
        let (audio, _rx) = AudioHandle::new();
        let mut state = crate::panel::Stage4PanelState::default();
        let mut library = BackupLibraryState::new();
        let mut firmware = Fw::new(Default::default());
        let mut os_update = OsUpdatePanel::new(None, None);
        let mut dlg = DebugDialogState::default();
        dlg.open_tab(DebugTab::OsUpdate);
        let mut all = String::new();
        for _ in 0..2 {
            let out = ctx.run(Default::default(), |ctx| {
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
            all.push_str(&texts(&out));
        }
        assert!(all.contains("Engine (experimental)"), "dialog text:\n{all}");
        assert!(all.contains("Saved choice:"));
    }
}
