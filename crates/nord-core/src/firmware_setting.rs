//! The "Nord OS update file" setting shared by `ns4 firmware ...` and the GUI (D129): check a file,
//! remember it, report whether the WAVE tables load. Reads the file on demand; stores only its path.

use crate::render_opts_wave::WAVE_TABLES_ENV;
use crate::user_config;
#[cfg(test)]
use crate::user_config::UserConfig;
use std::path::{Path, PathBuf};

/// The vendor's Nord Stage 4 Downloads page, where the user picks the OS update for their platform
/// (Windows `.zip`, Mac `.dmg`). We link the page, not a file, so the user downloads from the
/// vendor themselves (`docs/LEGAL.md` section 2). VERIFIED: the page loaded and listed the v1.66
/// Windows and Mac updates on 2026-11-24, checked by hand in a browser (no download).
pub const DOWNLOADS_PAGE: &str = "https://www.nordkeyboards.com/products/nord-stage-4/downloads";
/// Shown when no update file is set.
pub const PLACEHOLDER_NOTE: &str =
    "No Nord OS update file set: the built-in placeholder wavetable is used for WAVE sounds.";

/// Where the effective path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `NS4_WAVE_TABLES`.
    Environment,
    /// The saved config file.
    Saved,
}

/// What is set and whether it works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The effective update file, with its origin; `None` = placeholder in use.
    pub path: Option<(PathBuf, Source)>,
    /// Number of WAVE tables read from it, or the plain-language reason it failed.
    pub tables: Option<Result<usize, String>>,
    /// The saved settings file could not be read (corrupt); the placeholder is in use.
    pub config_error: Option<String>,
}

/// Number of WAVE tables `path` yields (reads and drops them; installs nothing).
pub fn check(path: &Path) -> Result<usize, String> {
    crate::synth::prepare_wave_bank();
    crate::synth::tables_from_file(path, crate::synth::boot_tables()).map(|t| t.len())
}

/// The effective setting: the environment variable, else the config file at `config`.
pub fn status(env: Option<&str>, config: &Path) -> Status {
    let mut config_error = None;
    let path = match env.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => Some((PathBuf::from(v), Source::Environment)),
        None => match user_config::saved_default(None, config) {
            Ok(p) => p.map(|p| (p, Source::Saved)),
            Err(e) => {
                config_error = Some(e);
                None
            }
        },
    };
    let tables = path.as_ref().map(|(p, _)| check(p));
    Status {
        path,
        tables,
        config_error,
    }
}

/// Validates `path`, then remembers it in `config` (made absolute). Returns the table count.
pub fn set(path: &Path, config: &Path) -> Result<usize, String> {
    let n = check(path)?;
    let abs = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let shown = abs.to_string_lossy();
    let shown = shown.strip_prefix(r"\\?\").unwrap_or(&shown).to_string();
    // Keep the other saved settings (text size): only the path changes.
    let mut saved = user_config::load(config).unwrap_or_default();
    saved.os_update_path = Some(shown);
    user_config::save(config, &saved)?;
    Ok(n)
}

/// Forgets the saved path.
pub fn clear(config: &Path) -> Result<(), String> {
    let mut saved = user_config::load(config).unwrap_or_default();
    saved.os_update_path = None;
    user_config::save(config, &saved)
}

/// Plain-language text of a [`Status`] for the CLI and the GUI.
pub fn describe(s: &Status) -> String {
    let Some((path, src)) = &s.path else {
        let broken = s.config_error.as_ref().map_or(String::new(), |e| {
            format!(
                "PROBLEM: saved settings not readable ({e}).
"
            )
        });
        return format!(
            "{broken}{PLACEHOLDER_NOTE}\nGet the official update (Windows .zip or Mac .dmg) from the Nord Stage 4 Downloads page: {DOWNLOADS_PAGE}\nThen run: ns4 firmware set \"<downloaded file>\""
        );
    };
    let from = match src {
        Source::Environment => format!("environment variable {WAVE_TABLES_ENV}"),
        Source::Saved => "saved setting".to_string(),
    };
    let result = match &s.tables {
        Some(Ok(n)) => format!("OK: {n} WAVE tables read from it"),
        Some(Err(e)) => format!("PROBLEM: {e}\nThe placeholder wavetable is used instead."),
        None => String::new(),
    };
    format!("Nord OS update file ({from}): {}\n{result}", path.display())
}

/// FNV-1a 64-bit hash (a fingerprint for comparing containers, not a security hash).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

/// `container: <bytes> bytes, fnv1a64 <hex>; WAVE tables: <n>` for `path`: numbers only.
pub fn info(path: &Path) -> Result<String, String> {
    let raw = crate::os_update::read_capped(path)?;
    let c = crate::os_update::extract_firmware_container(&raw)?;
    Ok(format!(
        "container: {} bytes, fnv1a64 {:016x}; WAVE tables: {}",
        c.len(),
        fnv1a64(&c),
        check(path)?
    ))
}

/// `ns4 firmware set PATH | status | clear`, against `config` and the value of `NS4_WAVE_TABLES`.
pub fn run(args: &[String], config: &Path, env: Option<&str>) -> Result<String, String> {
    const USAGE: &str =
        "usage: ns4 firmware set <Nord OS update .exe|.zip|.dmg> | status | clear | info <file>";
    match args.first().map(String::as_str) {
        Some("set") => {
            let p = args.get(1).ok_or(USAGE)?;
            let n = set(Path::new(p), config)?;
            Ok(format!(
                "Saved. {n} WAVE tables read from {p}. Applies to every ns4 and nd-matrix-1 start (NS4_WAVE_TABLES or --synth-wave-tables still override it)."
            ))
        }
        Some("status") | None => Ok(describe(&status(env, config))),
        Some("info") => info(Path::new(args.get(1).ok_or(USAGE)?)),
        Some("clear") => clear(config).map(|_| "Saved update file forgotten.".to_string()),
        Some(_) => Err(USAGE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("ns4-fwset-{name}-{}", std::process::id()))
            .join("config.json")
    }

    #[test]
    fn nothing_set_means_placeholder_and_the_download_link() {
        let s = status(None, &cfg("none"));
        assert_eq!(
            s,
            Status {
                path: None,
                tables: None,
                config_error: None
            }
        );
        let t = describe(&s);
        assert!(t.contains("placeholder"), "{t}");
        assert!(t.contains(DOWNLOADS_PAGE), "{t}");
        assert!(!t.contains("/wt/documents/"), "{t}");
    }

    #[test]
    fn the_download_link_is_the_downloads_page_not_a_file() {
        assert_eq!(
            DOWNLOADS_PAGE,
            "https://www.nordkeyboards.com/products/nord-stage-4/downloads"
        );
        for deep in ["/wt/documents/", ".zip", ".dmg", ".exe"] {
            assert!(
                !DOWNLOADS_PAGE.contains(deep),
                "{DOWNLOADS_PAGE} has {deep}"
            );
        }
    }

    #[test]
    fn a_corrupt_config_shows_in_status() {
        let c = cfg("corrupt");
        std::fs::create_dir_all(c.parent().unwrap()).unwrap();
        std::fs::write(&c, "{ nope").unwrap();
        let s = status(None, &c);
        assert_eq!(s.path, None);
        let t = describe(&s);
        assert!(t.contains("PROBLEM") && t.contains("placeholder"), "{t}");
    }

    #[test]
    fn the_variable_wins_and_a_bad_file_is_reported_plainly() {
        let c = cfg("env");
        let bad = std::env::temp_dir().join(format!("ns4-fwset-bad-{}.bin", std::process::id()));
        std::fs::write(&bad, b"not an update").unwrap();
        user_config::save(
            &c,
            &UserConfig {
                os_update_path: Some("/saved".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let s = status(Some(bad.to_str().unwrap()), &c);
        assert_eq!(s.path.as_ref().unwrap().1, Source::Environment);
        let t = describe(&s);
        assert!(
            t.contains("PROBLEM") && t.contains("not a Nord OS update"),
            "{t}"
        );
    }

    #[test]
    fn set_remembers_a_good_file_and_status_reports_it() {
        let c = cfg("good");
        let img = std::env::temp_dir().join(format!("ns4-fwset-good-{}.bin", std::process::id()));
        std::fs::write(&img, crate::synth::fw_wave_tables::tests::image()).unwrap();
        let out = run(&["set".into(), img.display().to_string()], &c, None).unwrap();
        assert!(out.contains("46 WAVE tables"), "{out}");
        let s = status(None, &c);
        assert_eq!(s.path.as_ref().unwrap().1, Source::Saved);
        assert_eq!(s.tables, Some(Ok(46)));
        assert!(describe(&s).contains("OK: 46 WAVE tables"));
    }

    #[test]
    fn set_keeps_the_saved_text_size() {
        let c = cfg("keep-text-size");
        user_config::save(
            &c,
            &UserConfig {
                text_scale_percent: Some(175),
                ..Default::default()
            },
        )
        .unwrap();
        let img = std::env::temp_dir().join(format!("ns4-fwset-keep-{}.bin", std::process::id()));
        std::fs::write(&img, crate::synth::fw_wave_tables::tests::image()).unwrap();
        set(&img, &c).unwrap();
        let saved = user_config::load(&c).unwrap();
        assert_eq!(saved.text_scale_percent, Some(175));
        assert!(saved.os_update_path.is_some());
        clear(&c).unwrap();
        let cleared = user_config::load(&c).unwrap();
        assert_eq!(cleared.text_scale_percent, Some(175));
        assert_eq!(cleared.os_update_path, None);
    }

    #[test]
    fn set_refuses_a_bad_file_and_stores_nothing() {
        let c = cfg("set-bad");
        let bad = std::env::temp_dir().join(format!("ns4-fwset-bad2-{}.bin", std::process::id()));
        std::fs::write(&bad, b"nope").unwrap();
        assert!(set(&bad, &c).is_err());
        assert!(!c.exists());
        let e = run(&["set".into(), bad.display().to_string()], &c, None).unwrap_err();
        assert!(e.contains("not a Nord OS update"), "{e}");
    }

    #[test]
    fn info_prints_only_numbers_for_the_container() {
        let img = std::env::temp_dir().join(format!("ns4-fwset-info-{}.bin", std::process::id()));
        std::fs::write(&img, crate::synth::fw_wave_tables::tests::image()).unwrap();
        // a raw bank0_CODE image has no container: the error says so
        assert!(info(&img).unwrap_err().contains("not a Nord OS update"));
        assert_eq!(fnv1a64(b""), 0xCBF2_9CE4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xAF63_DC4C_8601_EC8C);
    }

    #[test]
    fn clear_forgets_and_unknown_subcommands_print_usage() {
        let c = cfg("clear");
        user_config::save(
            &c,
            &UserConfig {
                os_update_path: Some("/x".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(run(&["clear".into()], &c, None).is_ok());
        assert_eq!(status(None, &c).path, None);
        assert!(run(&["bogus".into()], &c, None)
            .unwrap_err()
            .contains("usage"));
        assert!(run(&["set".into()], &c, None)
            .unwrap_err()
            .contains("usage"));
        assert!(run(&[], &c, None).unwrap().contains("placeholder"));
    }
}
