//! The worker's messages: stderr plus `plugin.log` next to the saved settings (a DAW shows
//! neither the console nor the plugin's threads).

use std::path::PathBuf;

/// `plugin.log` is emptied when it is larger than this at the next message. ASSUMED: a year of
/// sessions stays well under it.
const LOG_CAP: u64 = 1 << 20;

/// Where the worker's messages go besides stderr (a DAW shows neither): `plugin.log` next to
/// the saved settings (`%APPDATA%\ns4\plugin.log` on Windows), appended; emptied once it
/// passes [`LOG_CAP`].
pub fn log_path() -> Option<PathBuf> {
    Some(
        nord_core::user_config::default_config_path()?
            .parent()?
            .join("plugin.log"),
    )
}

/// Prints `msg` on stderr and appends it to [`log_path`] (silently when that fails).
pub(crate) fn report(msg: &str) {
    eprintln!("nd matrix 1: {msg}");
    if let Some(p) = log_path() {
        append_capped(&p, msg, LOG_CAP);
    }
}

/// Appends a line to `path`, emptying the file first when it is already larger than `cap`.
fn append_capped(path: &std::path::Path, msg: &str, cap: u64) {
    use std::io::Write;
    let oversized = std::fs::metadata(path).is_ok_and(|m| m.len() > cap);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(!oversized)
        .write(true)
        .truncate(oversized)
        .open(path);
    if let Ok(mut f) = file {
        let _ = writeln!(f, "{msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_log_sits_next_to_the_settings_and_is_emptied_past_the_cap() {
        if let Some(p) = log_path() {
            assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("plugin.log"));
            assert_eq!(
                p.parent(),
                nord_core::user_config::default_config_path()
                    .as_deref()
                    .and_then(Path::parent)
            );
        }
        let log = std::env::temp_dir().join(format!("nd-autolib-log-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&log);
        append_capped(&log, "one", 16);
        append_capped(&log, "two", 16);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "one\ntwo\n");
        append_capped(&log, "three", 6);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "three\n");
        let _ = std::fs::remove_file(&log);
    }
}
