//! The "sound library" setting shared by `ns4 library ...` and the plugin: which `.ns4b` backup
//! holds the programs, pianos and sample banks a DAW instance loads by its `library_program`
//! parameter (hosts without a plugin window never reach the editor's OPEN .NS4B). Like the Nord OS
//! update setting (D129) only the path is stored; the archive is read on demand.

use crate::cbin::Ns4BackupArchive;
pub use crate::firmware_setting::Source;
use crate::user_config;
use std::path::{Path, PathBuf};

/// Environment variable naming the library file; wins over the saved path.
pub const LIBRARY_ENV: &str = "NS4_LIBRARY";

/// `NS4_LIBRARY=none`: ignore the saved library in this process (tests, a host that must stay
/// quick to load).
pub const NO_LIBRARY: &str = "none";

/// Shown when no library is set.
pub const NONE_NOTE: &str =
    "No sound library set: the plugin plays its built-in sounds until a program is loaded.";

/// What is set and whether it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The effective library file with its origin; `None` = nothing set.
    pub path: Option<(PathBuf, Source)>,
    /// Number of programs in it, or the plain-language reason it cannot be used.
    pub programs: Option<Result<usize, String>>,
    /// The saved settings file could not be read (corrupt).
    pub config_error: Option<String>,
}

/// Number of programs (`.ns4p` entries) in the backup at `path`; reads only the archive index.
pub fn check(path: &Path) -> Result<usize, String> {
    if !path.is_file() {
        return Err(format!("{}: file not found", path.display()));
    }
    let archive = Ns4BackupArchive::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let n = archive.entries_with_ext(".ns4p").len();
    if n == 0 {
        return Err(format!("{}: this backup holds no programs", path.display()));
    }
    Ok(n)
}

/// The library to use now: `env` (the value of `NS4_LIBRARY`) when set and non-empty, else the
/// saved path in `config`. A damaged config counts as nothing saved (the plugin never fails to
/// start over it).
pub fn effective_path(env: Option<&str>, config: Option<&Path>) -> Option<PathBuf> {
    if let Some(v) = env.map(str::trim).filter(|v| !v.is_empty()) {
        if v.eq_ignore_ascii_case(NO_LIBRARY) {
            return None;
        }
        return Some(PathBuf::from(v));
    }
    user_config::saved_library(config?).ok().flatten()
}

/// The effective setting: the environment variable, else the config file at `config`.
pub fn status(env: Option<&str>, config: &Path) -> Status {
    let mut config_error = None;
    let path = match env.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => Some((PathBuf::from(v), Source::Environment)),
        None => match user_config::saved_library(config) {
            Ok(p) => p.map(|p| (p, Source::Saved)),
            Err(e) => {
                config_error = Some(e);
                None
            }
        },
    };
    let programs = path.as_ref().map(|(p, _)| check(p));
    Status {
        path,
        programs,
        config_error,
    }
}

/// Validates `path`, then remembers it in `config` (made absolute). Returns the program count.
pub fn set(path: &Path, config: &Path) -> Result<usize, String> {
    let n = check(path)?;
    let abs = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let shown = abs.to_string_lossy();
    let shown = shown.strip_prefix(r"\\?\").unwrap_or(&shown).to_string();
    user_config::update(config, |c| c.library_path = Some(shown))?;
    Ok(n)
}

/// Forgets the saved library (the Nord OS update setting stays).
pub fn clear(config: &Path) -> Result<(), String> {
    user_config::update(config, |c| c.library_path = None)
}

/// Plain-language text of a [`Status`] for the CLI.
pub fn describe(s: &Status) -> String {
    let Some((path, src)) = &s.path else {
        let broken = s.config_error.as_ref().map_or(String::new(), |e| {
            format!("PROBLEM: saved settings not readable ({e}).\n")
        });
        return format!("{broken}{NONE_NOTE}\nThen run: ns4 library set \"<your .ns4b backup>\"");
    };
    let from = match src {
        Source::Environment => format!("environment variable {LIBRARY_ENV}"),
        Source::Saved => "saved setting".to_string(),
    };
    let result = match &s.programs {
        Some(Ok(n)) => format!("OK: {n} programs in it"),
        Some(Err(e)) => format!("PROBLEM: {e}\nThe plugin plays its built-in sounds instead."),
        None => String::new(),
    };
    format!("Sound library ({from}): {}\n{result}", path.display())
}

/// `ns4 library set PATH | status | clear`, against `config` and the value of `NS4_LIBRARY`.
pub fn run(args: &[String], config: &Path, env: Option<&str>) -> Result<String, String> {
    const USAGE: &str = "usage: ns4 library set <backup.ns4b> | status | clear";
    match args.first().map(String::as_str) {
        Some("set") => {
            let p = args.get(1).ok_or(USAGE)?;
            let n = set(Path::new(p), config)?;
            Ok(format!(
                "Saved. {n} programs in {p}. Every new plugin instance loads its Program from it (NS4_LIBRARY still overrides it)."
            ))
        }
        Some("status") | None => Ok(describe(&status(env, config))),
        Some("clear") => clear(config).map(|_| "Saved sound library forgotten.".to_string()),
        Some(_) => Err(USAGE.to_string()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cbin::ProgramDecoded;
    use crate::user_config::UserConfig;

    /// A one-program backup on disk, at bank 0 location 0.
    pub(crate) fn one_program_backup(name: &str) -> PathBuf {
        let bytes = ProgramDecoded::default()
            .to_cbin_file(0, 0)
            .expect("encode")
            .to_bytes();
        let path = std::env::temp_dir().join(format!("ns4-lib-{name}-{}.ns4b", std::process::id()));
        let zip = crate::cbin::stored_archive(&[("Programs/Test.ns4p", &bytes)]).expect("archive");
        std::fs::write(&path, zip).unwrap();
        path
    }

    fn cfg(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("ns4-libset-{name}-{}", std::process::id()))
            .join("config.json")
    }

    #[test]
    fn set_counts_the_programs_and_keeps_the_os_update_setting() {
        let c = cfg("set");
        user_config::save(
            &c,
            &UserConfig {
                os_update_path: Some("/os.zip".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let bank = one_program_backup("set");
        let out = run(&["set".into(), bank.display().to_string()], &c, None).unwrap();
        assert!(out.starts_with("Saved. 1 programs"), "{out}");
        let saved = user_config::load(&c).unwrap();
        assert_eq!(saved.os_update_path.as_deref(), Some("/os.zip"));
        assert!(saved.library_path.is_some());
        let s = status(None, &c);
        assert_eq!(s.path.as_ref().map(|p| p.1), Some(Source::Saved));
        assert_eq!(s.programs, Some(Ok(1)));
        assert!(describe(&s).contains("OK: 1 programs"));
        assert_eq!(effective_path(None, Some(&c)), s.path.map(|p| p.0));
        assert!(run(&["clear".into()], &c, None).is_ok());
        assert_eq!(status(None, &c).path, None);
        assert_eq!(
            user_config::load(&c).unwrap().os_update_path.as_deref(),
            Some("/os.zip")
        );
        let _ = std::fs::remove_file(bank);
    }

    #[test]
    fn the_variable_wins_and_bad_files_are_reported_plainly() {
        let c = cfg("env");
        assert_eq!(
            effective_path(Some(" /env.ns4b "), None),
            Some("/env.ns4b".into())
        );
        assert_eq!(effective_path(None, None), None);
        assert_eq!(effective_path(Some("NONE"), None), None);
        let s = status(Some("/missing.ns4b"), &c);
        assert_eq!(s.path.as_ref().map(|p| p.1), Some(Source::Environment));
        assert!(describe(&s).contains("PROBLEM"), "{}", describe(&s));
        let not_zip =
            std::env::temp_dir().join(format!("ns4-lib-notzip-{}.ns4b", std::process::id()));
        std::fs::write(&not_zip, b"not a backup").unwrap();
        assert!(set(&not_zip, &c).is_err());
        let _ = std::fs::remove_file(not_zip);
        assert!(describe(&status(None, &c)).contains(NONE_NOTE));
        assert!(run(&["bogus".into()], &c, None).is_err());
        std::fs::create_dir_all(c.parent().unwrap()).unwrap();
        std::fs::write(&c, "{ not json").unwrap();
        let s = status(None, &c);
        assert!(s.config_error.is_some() && s.path.is_none());
        assert_eq!(effective_path(None, Some(&c)), None);
    }
}
