//! Per-user remembered choices for the front ends (D129): the **path** of the user's Nord OS
//! update file and the GUI text size (percent), in `<config dir>/ns4/config.json`. The firmware and
//! the tables read from it are never stored (D22). Library code and tests never read this file on
//! their own: the `ns4` and `nd-matrix-1` binaries call [`apply_saved_default`] once at start-up,
//! and the front ends load the text size once when a window opens (`nord_gui::text_scale`).
//!
//! Precedence for the WAVE-bank source, first match wins: `--synth-wave-tables PATH`, then
//! `NS4_WAVE_TABLES`, then this file, then the placeholder table. The flag already wins inside
//! `render_opts`; [`apply_saved_default`] only fills the environment variable when it is unset, so
//! the other rules fall out unchanged.

use crate::render_opts_wave::WAVE_TABLES_ENV;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The saved settings.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfig {
    /// Path of the user's Nord OS update file (`.exe`, `.dmg`, `.zip` or extracted container).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_update_path: Option<String>,
    /// Engine to start with: `hle` or `lle` (opt-in, see `engine_select`). Kept as text so an
    /// unknown value from a newer version never makes the whole file unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// SHARC float precision of the LLE engine: `exact` (default) or `fast` (see `precision`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<String>,
    /// Text size of the front ends in percent (100 to 200), `nord_gui::text_scale` (issue #592).
    /// Absent means 100. The front ends clamp and snap it; this file only stores it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_percent"
    )]
    pub text_scale_percent: Option<u32>,
}

/// A value that is not a non-negative whole number (`-5`, `150.5`, `"big"`, `null`) reads as absent,
/// so a bad text size can never make the whole file unreadable (and `save` then drop the Nord OS
/// update path next to it).
fn lenient_percent<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_u64().and_then(|n| u32::try_from(n).ok()))
}

/// Config directory for the given platform inputs (pure, for tests): Windows `%APPDATA%`, macOS
/// `~/Library/Application Support`, elsewhere `$XDG_CONFIG_HOME` or `~/.config`.
pub fn config_path_from(
    windows: bool,
    macos: bool,
    appdata: Option<&str>,
    xdg: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    let some = |v: Option<&str>| {
        v.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    };
    let base = if windows {
        some(appdata)?
    } else if macos {
        some(home)?.join("Library").join("Application Support")
    } else {
        some(xdg).or_else(|| some(home).map(|h| h.join(".config")))?
    };
    Some(base.join("ns4").join("config.json"))
}

/// The config file of the current user, or `None` when the platform gives no home.
pub fn default_config_path() -> Option<PathBuf> {
    let var = |k: &str| std::env::var(k).ok();
    config_path_from(
        cfg!(windows),
        cfg!(target_os = "macos"),
        var("APPDATA").as_deref(),
        var("XDG_CONFIG_HOME").as_deref(),
        var("HOME").or_else(|| var("USERPROFILE")).as_deref(),
    )
}

/// Loads `path`; a missing file is the empty config, an unreadable or malformed one an error.
pub fn load(path: &Path) -> Result<UserConfig, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(UserConfig::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Writes `cfg` to `path`, creating the directory.
pub fn save(path: &Path, cfg: &UserConfig) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

/// Loads `path`, applies `f` and writes it back, so one setting never erases the others. An
/// unreadable existing file is an error (never overwritten silently).
pub fn update(path: &Path, f: impl FnOnce(&mut UserConfig)) -> Result<(), String> {
    let mut cfg = load(path)?;
    f(&mut cfg);
    save(path, &cfg)
}

/// The saved update-file path that applies now: `Ok(None)` when `env` (the value of
/// `NS4_WAVE_TABLES`) is set and non-empty (the variable wins) or nothing is saved, else the path in
/// the config file at `config`. A config file that cannot be read or parsed is an `Err` (never
/// silently ignored).
pub fn saved_default(env: Option<&str>, config: &Path) -> Result<Option<PathBuf>, String> {
    if env.is_some_and(|v| !v.trim().is_empty()) {
        return Ok(None);
    }
    Ok(load(config)?
        .os_update_path
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from))
}

/// What start-up does with the saved setting: the path to put in `NS4_WAVE_TABLES` (only when it
/// works), and a plain-language warning when a saved setting had to be skipped.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SavedApplication {
    /// Saved update file that loads; `None` means the placeholder wavetable is used.
    pub path: Option<PathBuf>,
    /// Why a saved setting was skipped (moved, deleted, corrupt config or file).
    pub warning: Option<String>,
}

/// Decides what to apply for `env` and the config file at `config`. A saved path is applied only
/// when it still loads (`check` = `firmware_setting::check`); otherwise the placeholder stays and a
/// warning says why, so a moved download never stops the app or `ns4` from starting (the explicit
/// variable and flag keep failing loudly: they never go through here).
pub fn resolve_saved(
    env: Option<&str>,
    config: &Path,
    check: impl Fn(&Path) -> Result<usize, String>,
) -> SavedApplication {
    match saved_default(env, config) {
        Ok(None) => SavedApplication::default(),
        Ok(Some(p)) => match check(&p) {
            Ok(_) => SavedApplication {
                path: Some(p),
                warning: None,
            },
            Err(e) => SavedApplication {
                path: None,
                warning: Some(format!(
                    "saved Nord OS update file not used ({e}); the placeholder wavetable is used. Fix with: ns4 firmware set \"<file>\""
                )),
            },
        },
        Err(e) => SavedApplication {
            path: None,
            warning: Some(format!(
                "saved settings not used ({e}); the placeholder wavetable is used"
            )),
        },
    }
}

/// Binaries only, once, before any thread starts and before options are parsed: fills
/// `NS4_WAVE_TABLES` from the saved config when the variable is unset and the saved file still
/// loads. A saved file that moved or is damaged prints a warning on stderr and leaves the
/// placeholder in place. Returns the path applied.
pub fn apply_saved_default() -> Option<PathBuf> {
    let cfg = default_config_path()?;
    let env = std::env::var(WAVE_TABLES_ENV).ok();
    let applied = resolve_saved(env.as_deref(), &cfg, crate::firmware_setting::check);
    if let Some(w) = &applied.warning {
        eprintln!("ns4: {w}");
    }
    if let Some(p) = &applied.path {
        std::env::set_var(WAVE_TABLES_ENV, p);
    }
    applied.path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ns4-cfg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("ns4").join("config.json")
    }

    #[test]
    fn the_config_dir_follows_each_platform() {
        let p = |w, m, a, x, h| config_path_from(w, m, a, x, h);
        assert_eq!(
            p(true, false, Some("C:/Users/u/AppData/Roaming"), None, None),
            Some(PathBuf::from("C:/Users/u/AppData/Roaming/ns4/config.json"))
        );
        assert_eq!(
            p(false, true, None, None, Some("/Users/u")),
            Some(PathBuf::from(
                "/Users/u/Library/Application Support/ns4/config.json"
            ))
        );
        assert_eq!(
            p(false, false, None, Some("/x"), Some("/home/u")),
            Some(PathBuf::from("/x/ns4/config.json"))
        );
        assert_eq!(
            p(false, false, None, Some(" "), Some("/home/u")),
            Some(PathBuf::from("/home/u/.config/ns4/config.json"))
        );
        assert_eq!(p(false, false, None, None, None), None);
    }

    #[test]
    fn save_then_load_round_trips_and_a_missing_file_is_empty() {
        let path = tmp("roundtrip");
        assert_eq!(load(&path).unwrap(), UserConfig::default());
        let cfg = UserConfig {
            os_update_path: Some("C:/dl/Update.exe".into()),
            ..Default::default()
        };
        save(&path, &cfg).unwrap();
        assert_eq!(load(&path).unwrap(), cfg);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(load(&path).unwrap_err().contains("config.json"));
    }

    #[test]
    fn update_keeps_the_other_settings_and_refuses_a_corrupt_file() {
        let path = tmp("update");
        update(&path, |c| c.engine = Some("lle".into())).unwrap();
        update(&path, |c| c.os_update_path = Some("/os.exe".into())).unwrap();
        let cfg = load(&path).unwrap();
        assert_eq!(cfg.engine.as_deref(), Some("lle"));
        assert_eq!(cfg.os_update_path.as_deref(), Some("/os.exe"));
        std::fs::write(&path, "{ not json").unwrap();
        assert!(update(&path, |c| c.engine = None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn the_text_size_round_trips_and_an_old_file_without_it_still_loads() {
        let path = tmp("textscale");
        let cfg = UserConfig {
            text_scale_percent: Some(150),
            ..Default::default()
        };
        save(&path, &cfg).unwrap();
        assert_eq!(load(&path).unwrap().text_scale_percent, Some(150));
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("os_update_path"));
        std::fs::write(&path, r#"{ "os_update_path": "C:/a.exe" }"#).unwrap();
        let old = load(&path).unwrap();
        assert_eq!(old.text_scale_percent, None);
        assert_eq!(old.os_update_path.as_deref(), Some("C:/a.exe"));
    }

    #[test]
    fn a_bad_text_size_is_ignored_and_the_other_settings_survive() {
        let path = tmp("badsize");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bad in ["-5", "150.5", "\"big\"", "null", "99999999999", "[1]"] {
            let text =
                format!(r#"{{ "os_update_path": "C:/a.exe", "text_scale_percent": {bad} }}"#);
            std::fs::write(&path, text).unwrap();
            let cfg = load(&path).unwrap_or_else(|e| panic!("{bad}: {e}"));
            assert_eq!(cfg.text_scale_percent, None, "{bad}");
            assert_eq!(cfg.os_update_path.as_deref(), Some("C:/a.exe"), "{bad}");
        }
    }

    #[test]
    fn the_variable_wins_over_the_saved_path() {
        let path = tmp("precedence");
        save(
            &path,
            &UserConfig {
                os_update_path: Some("/saved/os.exe".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            saved_default(None, &path),
            Ok(Some(PathBuf::from("/saved/os.exe")))
        );
        assert_eq!(
            saved_default(Some(""), &path),
            Ok(Some(PathBuf::from("/saved/os.exe")))
        );
        assert_eq!(saved_default(Some("/env/os.exe"), &path), Ok(None));
        assert_eq!(
            saved_default(None, Path::new("no/such/config.json")),
            Ok(None)
        );
    }

    #[test]
    fn an_unset_path_is_not_written() {
        let path = tmp("empty");
        save(&path, &UserConfig::default()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "{}");
        assert_eq!(saved_default(None, &path), Ok(None));
    }

    #[test]
    fn a_corrupt_config_is_reported_not_ignored() {
        let path = tmp("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert!(saved_default(None, &path)
            .unwrap_err()
            .contains("config.json"));
        let r = resolve_saved(None, &path, |_| Ok(46));
        assert_eq!(r.path, None);
        assert!(r.warning.unwrap().contains("placeholder"));
        // the variable still wins without touching the broken file
        assert_eq!(
            resolve_saved(Some("/env"), &path, |_| Ok(46)),
            Default::default()
        );
    }

    #[test]
    fn a_moved_saved_file_falls_back_to_the_placeholder_with_a_message() {
        let path = tmp("stale");
        let gone = std::env::temp_dir().join(format!("ns4-cfg-gone-{}.exe", std::process::id()));
        let _ = std::fs::remove_file(&gone);
        let cfg = UserConfig {
            os_update_path: Some(gone.display().to_string()),
            ..Default::default()
        };
        save(&path, &cfg).unwrap();
        let r = resolve_saved(None, &path, crate::firmware_setting::check);
        assert_eq!(r.path, None, "a stale path must not reach NS4_WAVE_TABLES");
        let w = r.warning.expect("a visible message");
        assert!(
            w.contains("placeholder") && w.contains("ns4 firmware set"),
            "{w}"
        );
        // a damaged (wrong) file falls back the same way
        std::fs::write(&gone, b"not an update").unwrap();
        let r = resolve_saved(None, &path, crate::firmware_setting::check);
        assert_eq!(r.path, None);
        assert!(r.warning.unwrap().contains("not a Nord OS update"));
    }

    #[test]
    fn a_saved_file_that_loads_is_applied_without_a_warning() {
        let path = tmp("good");
        let img = std::env::temp_dir().join(format!("ns4-cfg-good-{}.bin", std::process::id()));
        std::fs::write(&img, crate::synth::fw_wave_tables::tests::image()).unwrap();
        save(
            &path,
            &UserConfig {
                os_update_path: Some(img.display().to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let r = resolve_saved(None, &path, crate::firmware_setting::check);
        assert_eq!(r.path, Some(img));
        assert_eq!(r.warning, None);
    }
}
