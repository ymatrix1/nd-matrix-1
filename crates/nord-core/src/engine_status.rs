//! What the engine selector shows the user (bean `ns4-lle7`, D246): the saved choice, the
//! `NS4_ENGINE` override, the engine that runs this session and whether the LLE engine could run
//! here. Pure functions over explicit inputs (no process state), so the GUI and its tests call the
//! same code as the start-up path. The decision itself stays in [`engine_select::resolve`]; this
//! module only reads the inputs leniently and explains the result.
//!
//! An unknown value never aborts and never switches silently: it reads as the built-in engine (HLE)
//! and the note names the value and where it came from.

use crate::engine_select::{self, EngineChoice, EngineKind, LleAvailability, LleBackend};
use std::path::Path;

/// One reading of the engine settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineStatus {
    /// The saved choice (HLE when absent or unknown).
    pub saved: EngineKind,
    /// `NS4_ENGINE` when it holds a valid value.
    pub env_override: Option<EngineKind>,
    /// What runs and why it may differ from the request.
    pub choice: EngineChoice,
    /// Whether the LLE engine exists in this build (ignores the update file).
    pub backend: LleAvailability,
    /// A Nord OS update file is set and exists.
    pub os_update_present: bool,
    /// Unknown or unreadable inputs that were ignored, one plain sentence each.
    pub notes: Vec<String>,
}

/// Parses one level leniently: `None` for absent or empty, HLE plus a note for an unknown value.
fn lenient(value: Option<&str>, what: &str, notes: &mut Vec<String>) -> Option<EngineKind> {
    let v = value.map(str::trim).filter(|v| !v.is_empty())?;
    match EngineKind::parse(v) {
        Ok(k) => Some(k),
        Err(e) => {
            notes.push(format!("{what}: {e}. The built-in engine is used."));
            Some(EngineKind::Hle)
        }
    }
}

/// Reads the environment value, the saved value and the update file (`None` = not set) with
/// `probe` as the LLE availability check. Precedence: environment, saved value, HLE (the
/// `--engine` flag is read at start-up by `engine_select::startup`, not here).
pub fn status_with(
    env: Option<&str>,
    saved: Option<&str>,
    os_update: Option<&Path>,
    probe: impl Fn() -> LleAvailability,
) -> EngineStatus {
    let mut notes = Vec::new();
    let env_override = lenient(env, engine_select::ENGINE_ENV, &mut notes);
    let saved_kind = lenient(saved, "saved engine setting", &mut notes).unwrap_or_default();
    let requested = env_override.unwrap_or(saved_kind);
    let choice = engine_select::resolve(requested, os_update, &probe);
    EngineStatus {
        saved: saved_kind,
        env_override,
        choice,
        backend: probe(),
        os_update_present: os_update.is_some(),
        notes,
    }
}

/// [`status_with`] for the running process: reads `NS4_ENGINE` and the real backend probe.
pub fn status(saved: Option<&str>, os_update: Option<&Path>) -> EngineStatus {
    let env = std::env::var(engine_select::ENGINE_ENV).ok();
    status_with(env.as_deref(), saved, os_update, LleBackend::probe)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: fn() -> LleAvailability = || LleAvailability::Available;
    const DOWN: fn() -> LleAvailability = || LleAvailability::Unavailable("no engine".into());

    #[test]
    fn nothing_set_is_the_built_in_engine_without_notes() {
        let s = status_with(None, None, None, DOWN);
        assert_eq!(s.saved, EngineKind::Hle);
        assert_eq!(s.env_override, None);
        assert_eq!(s.choice.effective, EngineKind::Hle);
        assert!(s.notes.is_empty() && !s.os_update_present);
    }

    #[test]
    fn the_environment_beats_the_saved_choice_which_beats_the_default() {
        let f = Path::new("os.zip");
        let s = status_with(Some("lle"), Some("hle"), Some(f), UP);
        assert_eq!(s.saved, EngineKind::Hle);
        assert_eq!(s.env_override, Some(EngineKind::Lle));
        assert_eq!(s.choice.effective, EngineKind::Lle);
        let s = status_with(Some("hle"), Some("lle"), Some(f), UP);
        assert_eq!(s.choice.effective, EngineKind::Hle);
        let s = status_with(None, Some("lle"), Some(f), UP);
        assert_eq!(s.choice.effective, EngineKind::Lle);
        let s = status_with(Some("  "), Some("lle"), Some(f), UP);
        assert_eq!(s.env_override, None);
    }

    #[test]
    fn an_unavailable_engine_falls_back_with_the_reason() {
        let f = Path::new("os.zip");
        let s = status_with(None, Some("lle"), Some(f), DOWN);
        assert_eq!(s.choice.requested, EngineKind::Lle);
        assert_eq!(s.choice.effective, EngineKind::Hle);
        assert!(s.choice.notice.unwrap().contains("no engine"));
        assert_eq!(s.backend, LleAvailability::Unavailable("no engine".into()));
        let s = status_with(None, Some("lle"), None, UP);
        assert_eq!(s.choice.effective, EngineKind::Hle);
        assert!(s.choice.notice.unwrap().contains("Nord OS update file"));
        assert_eq!(s.backend, LleAvailability::Available);
    }

    #[test]
    fn unknown_values_read_as_hle_and_say_where_they_came_from() {
        let s = status_with(Some("jit"), Some("turbo"), None, DOWN);
        assert_eq!(s.choice.effective, EngineKind::Hle);
        assert_eq!(s.notes.len(), 2);
        assert!(s.notes[0].contains("NS4_ENGINE") && s.notes[0].contains("jit"));
        assert!(s.notes[1].contains("saved engine") && s.notes[1].contains("turbo"));
        // a bad environment value is a request for HLE, so a saved LLE does not take over
        let s = status_with(Some("jit"), Some("lle"), Some(Path::new("o")), UP);
        assert_eq!(s.choice.effective, EngineKind::Hle);
    }

    #[test]
    fn the_process_reading_runs_the_real_probe() {
        // works whatever NS4_ENGINE holds in the test process
        let s = status(Some("lle"), Some(Path::new("os.zip")));
        assert_eq!(
            s.choice.requested,
            s.env_override.unwrap_or(EngineKind::Lle)
        );
        assert_eq!(s.backend, LleBackend::probe());
    }
}
