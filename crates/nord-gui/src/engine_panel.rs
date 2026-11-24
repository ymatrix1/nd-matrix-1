//! "Engine" toggle of the Settings tab (bean `ns4-lle7`): built-in engine (HLE, default) or the
//! opt-in low-level engine (LLE). Shared by `nd-matrix-1` and the DAW plugin through
//! `os_update_panel`. It shows and edits the **saved** choice only: a `--engine` flag or
//! `NS4_ENGINE` is read by `engine_select::startup` in the binaries (the plugin has no such flag). The LLE engine does not exist yet, so choosing it shows why the built-in
//! engine still runs (`nord_core::engine_select`); the choice is saved and applies at next start.

use nord_core::engine_select::{self, EngineChoice, EngineKind, LleBackend};
use nord_core::precision::{self, PrecisionMode};
use std::path::{Path, PathBuf};

/// State of the engine toggle.
pub struct EnginePanel {
    config: Option<PathBuf>,
    requested: EngineKind,
    precision: PrecisionMode,
    message: Option<String>,
}

impl EnginePanel {
    /// Reads the saved choice from `config`; an unreadable file or unknown value keeps HLE and says so.
    pub fn new(config: Option<PathBuf>) -> Self {
        let mut message = None;
        let mut precision = PrecisionMode::Exact;
        let requested = match config.as_deref().map(nord_core::user_config::load) {
            Some(Ok(c)) => {
                let (p, notice) = precision::resolve(&[], None, c.precision.as_deref());
                precision = p;
                message = notice;
                match engine_select::requested(&[], None, c.engine.as_deref()) {
                    Ok(k) => k,
                    Err(e) => {
                        message = Some(format!("{e}. The built-in engine is used."));
                        EngineKind::Hle
                    }
                }
            }
            Some(Err(e)) => {
                message = Some(format!("saved settings not read ({e})."));
                EngineKind::Hle
            }
            None => EngineKind::Hle,
        };
        Self {
            config,
            requested,
            precision,
            message,
        }
    }

    /// The engine that runs for the saved request, given the effective update file.
    pub fn choice(&self, os_update: Option<&Path>) -> EngineChoice {
        let mut c = engine_select::resolve(self.requested, os_update, LleBackend::probe);
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
        self.message = Some(match &self.config {
            None => {
                "This system has no place to save settings; the choice lasts until you quit.".into()
            }
            Some(c) => match precision::save_choice(c, mode) {
                Ok(()) => "Saved. Restart the app to apply the precision choice.".into(),
                Err(e) => format!("Not saved: {e}"),
            },
        });
    }

    /// Chooses `kind` and saves it; the message says what happened in plain words.
    pub fn select(&mut self, kind: EngineKind) {
        self.requested = kind;
        self.message = Some(match &self.config {
            None => {
                "This system has no place to save settings; the choice lasts until you quit.".into()
            }
            Some(c) => match engine_select::save_choice(c, kind) {
                Ok(()) => "Saved. Restart the app to apply the engine choice.".into(),
                Err(e) => format!("Not saved: {e}"),
            },
        });
    }

    /// Draws the toggle and the fallback notice.
    pub fn show(&mut self, ui: &mut egui::Ui, os_update: Option<&Path>) {
        ui.label("Engine (experimental): built-in is the default. LLE runs your own Nord OS update file and is opt-in.");
        ui.label(
            "This is the saved choice; a --engine flag or NS4_ENGINE overrides it for one run.",
        );
        let mut pick = self.requested;
        ui.horizontal(|ui| {
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
        if let Some(n) = self.choice(os_update).notice {
            ui.label(n);
        }
        if let Some(m) = &self.message {
            ui.label(m);
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

    #[test]
    fn default_is_hle_without_a_notice() {
        let p = EnginePanel::new(Some(config("default")));
        let c = p.choice(None);
        assert_eq!((c.effective, c.notice), (EngineKind::Hle, None));
    }

    #[test]
    fn choosing_lle_saves_it_and_falls_back_with_a_prompt_then_the_reason() {
        let c = config("lle");
        let mut p = EnginePanel::new(Some(c.clone()));
        p.select(EngineKind::Lle);
        assert!(p.message.as_deref().unwrap().contains("Saved"));
        let again = EnginePanel::new(Some(c));
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
        let mut p = EnginePanel::new(Some(c.clone()));
        assert!(!p.precision_enabled());
        assert_eq!(p.choice(None).precision, PrecisionMode::Exact);
        p.select(EngineKind::Lle);
        assert!(p.precision_enabled());
        p.select_precision(PrecisionMode::Fast);
        let again = EnginePanel::new(Some(c.clone()));
        assert_eq!(again.choice(None).precision, PrecisionMode::Fast);
        // the engine key survived the precision save
        assert_eq!(again.requested, EngineKind::Lle);
        nord_core::user_config::update(&c, |u| u.precision = Some("half".into())).unwrap();
        let bad = EnginePanel::new(Some(c));
        assert_eq!(bad.choice(None).precision, PrecisionMode::Exact);
        assert!(bad.message.unwrap().contains("half"));
    }

    #[test]
    fn a_corrupt_or_unsaveable_config_is_reported_not_fatal() {
        let c = config("bad");
        std::fs::create_dir_all(c.parent().unwrap()).unwrap();
        std::fs::write(&c, "{ nope").unwrap();
        let p = EnginePanel::new(Some(c));
        assert_eq!(p.requested, EngineKind::Hle);
        assert!(p.message.unwrap().contains("not read"));
        let mut none = EnginePanel::new(None);
        none.select(EngineKind::Lle);
        assert!(none.message.unwrap().contains("no place to save"));
    }

    #[test]
    fn the_panel_draws_without_a_display() {
        let mut p = EnginePanel::new(Some(config("draw")));
        p.select(EngineKind::Lle);
        let ctx = egui::Context::default();
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| p.show(ui, None));
        });
        assert!(!out.shapes.is_empty());
        // LLE is saved but no update file is set: the prompt notice is what the panel shows
        assert!(p
            .choice(None)
            .notice
            .unwrap()
            .contains("Nord OS update file"));
    }
}
