//! The user's Nord OS update ("firmware") WAVE tables, installed into the process off the audio
//! thread (D215).
//!
//! The standalone app reads them at start-up (`user_config::apply_saved_default`, which sets an
//! environment variable: unsafe in a multi-threaded host) and installs them once per process
//! (`synth::install_wave_tables`, a `OnceLock`). The plugin reads the same saved path on a worker
//! thread when an instance activates or the settings dialog closes, then installs the parsed
//! tables; the engine reads them lock-free through `wave_table(id)`. The WAVE stage itself is the
//! opt-in `synth_wave_bank` option of the Firmware & DSP tab (`LoadJob::Firmware`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Environment variable that names the update file (the standalone and `ns4` read it too).
pub const WAVE_TABLES_ENV: &str = "NS4_WAVE_TABLES";

/// A worker was started (or tables are installed): later calls do nothing.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Parses the tables in the update file at `path` and installs them for this process. `Err` is a
/// plain-language reason; installing twice is a no-op (the first set wins, as in the standalone).
pub fn install_from(path: &Path) -> Result<usize, String> {
    nord_core::synth::prepare_wave_bank();
    let tables = nord_core::synth::tables_from_file(path, nord_core::synth::boot_tables())?;
    let n = tables.len();
    nord_core::synth::install_wave_tables(tables);
    Ok(n)
}

/// The update file to use: `NS4_WAVE_TABLES` when set, else the path in the saved settings.
pub fn saved_path(env: Option<&str>, config: Option<&Path>) -> Option<PathBuf> {
    if let Some(v) = env.map(str::trim).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(v));
    }
    nord_core::user_config::saved_default(None, config?)
        .ok()
        .flatten()
}

/// Starts the one-time background install of the saved update file. Returns `false` when it was
/// already started or there is nothing saved. Never blocks the caller.
pub fn install_saved_in_background() -> bool {
    let env = std::env::var(WAVE_TABLES_ENV).ok();
    let config = nord_core::user_config::default_config_path();
    let Some(path) = saved_path(env.as_deref(), config.as_deref()) else {
        return false;
    };
    if STARTED.swap(true, Ordering::AcqRel) {
        return false;
    }
    let spawned = std::thread::Builder::new()
        .name("ns4-wave-tables".into())
        .spawn(move || {
            if let Err(e) = install_from(&path) {
                eprintln!("nd matrix 1: Nord OS update file not used ({e})");
            }
        });
    if spawned.is_err() {
        STARTED.store(false, Ordering::Release);
    }
    spawned.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_config(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nd-wave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn the_environment_variable_wins_over_the_saved_path() {
        let cfg = tmp_config("env", r#"{"os_update_path":"saved.zip"}"#);
        assert_eq!(
            saved_path(Some(" env.zip "), Some(&cfg)),
            Some(PathBuf::from("env.zip"))
        );
        assert_eq!(
            saved_path(None, Some(&cfg)),
            Some(PathBuf::from("saved.zip"))
        );
        assert_eq!(
            saved_path(Some("  "), Some(&cfg)),
            Some(PathBuf::from("saved.zip"))
        );
    }

    #[test]
    fn nothing_saved_or_a_damaged_config_means_the_placeholder() {
        let bad = tmp_config("bad", "{not json");
        assert_eq!(saved_path(None, Some(&bad)), None);
        assert_eq!(saved_path(None, None), None);
        let empty = tmp_config("empty", "{}");
        assert_eq!(saved_path(None, Some(&empty)), None);
    }

    #[test]
    fn a_missing_update_file_is_reported_and_installs_nothing() {
        let err = install_from(Path::new("definitely-not-a-file.zip")).unwrap_err();
        assert!(!err.is_empty());
    }
}
