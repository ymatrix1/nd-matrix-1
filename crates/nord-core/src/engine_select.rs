//! Opt-in engine selection (bean `ns4-lle7`, D2 / D119 update in `docs/DECISIONS.md`).
//!
//! The high-level Rust DSP engine (HLE, D2) stays the default. The low-level engine (LLE: ARM0 plus
//! the SHARC DSPs running the user's own Nord OS update file) is opt-in only and does not exist yet:
//! [`LleBackend::probe`] is a stub that always says "unavailable". This module is the seam the real
//! backend plugs into later. It decides which engine runs and, whenever the answer differs from what
//! the user asked for, produces a plain-language notice. A request for LLE never panics and never
//! falls back silently.
//!
//! Precedence of the request, first match wins: `--engine hle|lle`, then `NS4_ENGINE`, then the
//! saved setting (`user_config::UserConfig::engine`), then HLE.

use crate::precision::PrecisionMode;
use crate::render_opts_wave::WAVE_TABLES_ENV;
use std::path::{Path, PathBuf};

/// Environment variable that selects the engine (below the flag, above the saved setting).
pub const ENGINE_ENV: &str = "NS4_ENGINE";

/// Which DSP engine renders audio.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    /// High-level Rust DSP (D2): the default, needs nothing from the user.
    #[default]
    Hle,
    /// Low-level emulation of ARM0 + SHARC DSPs from the user's own OS update file (opt-in).
    Lle,
}

impl EngineKind {
    /// Lower-case name used on the command line, in the environment and in the config file.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hle => "hle",
            Self::Lle => "lle",
        }
    }

    /// Parses `hle` or `lle` (case-insensitive); anything else is a plain-language error.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "hle" => Ok(Self::Hle),
            "lle" => Ok(Self::Lle),
            other => Err(format!("unknown engine \"{other}\" (use hle or lle)")),
        }
    }
}

/// Whether the LLE backend can run in this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LleAvailability {
    /// Ready to construct.
    Available,
    /// Cannot run; the string is the user-facing reason.
    Unavailable(String),
}

/// Availability probe of the LLE backend.
pub struct LleBackend;

impl LleBackend {
    /// STUB: the LLE engine is not implemented (epic `ns4-lle1`, beans `ns4-lle2`..`ns4-lle6`), so
    /// this always reports `Unavailable`. ASSUMED: the real probe will check the build feature and
    /// the loaded images; replace this body, not the callers.
    pub fn probe() -> LleAvailability {
        LleAvailability::Unavailable(
            "the low-level engine (LLE) is not implemented in this build yet".into(),
        )
    }
}

/// The engine that runs, and why it may differ from the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineChoice {
    /// What the user asked for.
    pub requested: EngineKind,
    /// What actually runs (always constructible: HLE whenever LLE cannot run).
    pub effective: EngineKind,
    /// Shown to the user when `effective != requested`; `None` otherwise.
    pub notice: Option<String>,
    /// SHARC float precision for the LLE engine (ignored by HLE); set by `precision::apply_startup`.
    pub precision: PrecisionMode,
}

/// Decides the engine for `requested`. `os_update` is the effective Nord OS update file (`None` =
/// not set), `probe` the LLE availability check. LLE needs both; otherwise HLE runs and the notice
/// says how to fix it.
pub fn resolve(
    requested: EngineKind,
    os_update: Option<&Path>,
    probe: impl FnOnce() -> LleAvailability,
) -> EngineChoice {
    let fallback = |why: String| EngineChoice {
        requested,
        effective: EngineKind::Hle,
        notice: Some(format!("{why} Using the built-in engine (HLE) instead.")),
        precision: PrecisionMode::Exact,
    };
    if requested == EngineKind::Hle {
        return EngineChoice {
            requested,
            effective: requested,
            notice: None,
            precision: PrecisionMode::Exact,
        };
    }
    if os_update.is_none() {
        return fallback(
            "The LLE engine needs your own Nord OS update file and none is set. Set it with: ns4 firmware set \"<file>\" (or Settings > OS & Wave Tables).".into(),
        );
    }
    match probe() {
        LleAvailability::Available => EngineChoice {
            requested,
            effective: EngineKind::Lle,
            notice: None,
            precision: PrecisionMode::Exact,
        },
        LleAvailability::Unavailable(reason) => {
            fallback(format!("The LLE engine cannot run: {reason}."))
        }
    }
}

/// The value after `--engine` in `args`: `Ok(None)` when the flag is absent, `Err` when the value is
/// missing or unknown.
pub fn from_args(args: &[String]) -> Result<Option<EngineKind>, String> {
    let Some(i) = args.iter().position(|a| a == "--engine") else {
        return Ok(None);
    };
    match args.get(i + 1) {
        Some(v) => EngineKind::parse(v).map(Some),
        None => Err("--engine needs a value (hle or lle)".into()),
    }
}

/// The requested engine from the flag, the environment value and the saved value (see the module
/// doc for the order). An unknown value at any level is an error naming it.
pub fn requested(
    args: &[String],
    env: Option<&str>,
    saved: Option<&str>,
) -> Result<EngineKind, String> {
    if let Some(k) = from_args(args)? {
        return Ok(k);
    }
    for (v, what) in [(env, ENGINE_ENV), (saved, "saved engine setting")] {
        if let Some(v) = v.map(str::trim).filter(|v| !v.is_empty()) {
            return EngineKind::parse(v).map_err(|e| format!("{what}: {e}"));
        }
    }
    Ok(EngineKind::Hle)
}

/// The effective update file from the value of `NS4_WAVE_TABLES` (the saved path was copied into it
/// at start-up when it still loads): `None` when unset, empty or naming a path that does not exist,
/// so a stale variable gets the "set your update file" prompt instead of a later failure.
pub fn os_update_from(env: Option<&str>) -> Option<PathBuf> {
    env.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// [`os_update_from`] for the running process.
pub fn effective_os_update() -> Option<PathBuf> {
    os_update_from(std::env::var(WAVE_TABLES_ENV).ok().as_deref())
}

/// `args` without every `--engine <value>` pair, for command parsers that do not know the flag
/// (the flag is read by [`startup`] first, so it can sit anywhere, even before the subcommand).
pub fn strip_flag(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--engine" {
            it.next();
        } else {
            out.push(a.clone());
        }
    }
    out
}

/// Remembers `kind` in the config file at `config` (other saved settings are kept).
pub fn save_choice(config: &Path, kind: EngineKind) -> Result<(), String> {
    crate::user_config::update(config, |c| c.engine = Some(kind.as_str().into()))
}

/// Binaries, once at start-up after `user_config::apply_saved_default`: reads the flag, environment
/// and saved setting, resolves the engine and prints any notice on stderr as `<who>: <notice>`. A bad
/// request value is reported the same way and HLE runs. Returns the choice for the caller to wire.
pub fn startup(who: &str, args: &[String]) -> EngineChoice {
    let mut saved = None;
    let mut config_error = None;
    if let Some(p) = crate::user_config::default_config_path() {
        match crate::user_config::load(&p) {
            Ok(c) => saved = c.engine,
            Err(e) => config_error = Some(e),
        }
    }
    let env = std::env::var(ENGINE_ENV).ok();
    let os_update = effective_os_update();
    startup_with(
        who,
        args,
        env.as_deref(),
        saved.as_deref(),
        config_error.as_deref(),
        os_update.as_deref(),
    )
}

/// [`startup`] with every input given (no process state), for tests.
pub fn startup_with(
    who: &str,
    args: &[String],
    env: Option<&str>,
    saved: Option<&str>,
    config_error: Option<&str>,
    os_update: Option<&Path>,
) -> EngineChoice {
    if let Some(e) = config_error {
        eprintln!("{who}: saved settings not read ({e}); the saved engine choice is ignored");
    }
    let choice = match requested(args, env, saved) {
        Ok(k) => resolve(k, os_update, LleBackend::probe),
        Err(e) => EngineChoice {
            requested: EngineKind::Hle,
            effective: EngineKind::Hle,
            notice: Some(format!("{e}. Using the built-in engine (HLE) instead.")),
            precision: PrecisionMode::Exact,
        },
    };
    if let Some(n) = &choice.notice {
        eprintln!("{who}: {n}");
    }
    choice
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn names_parse_both_ways_and_unknown_is_an_error() {
        assert_eq!(EngineKind::parse("LLE"), Ok(EngineKind::Lle));
        assert_eq!(EngineKind::parse(" hle "), Ok(EngineKind::Hle));
        assert_eq!(EngineKind::Lle.as_str(), "lle");
        assert_eq!(EngineKind::default(), EngineKind::Hle);
        assert!(EngineKind::parse("jit").unwrap_err().contains("jit"));
    }

    #[test]
    fn the_stub_probe_is_unavailable_with_a_reason() {
        match LleBackend::probe() {
            LleAvailability::Unavailable(r) => assert!(r.contains("not implemented")),
            LleAvailability::Available => panic!("the LLE engine does not exist yet"),
        }
    }

    #[test]
    fn hle_never_probes_or_warns() {
        let c = resolve(EngineKind::Hle, None, || panic!("must not probe"));
        assert_eq!(c.effective, EngineKind::Hle);
        assert_eq!(c.notice, None);
    }

    #[test]
    fn lle_without_an_update_file_falls_back_with_a_prompt_and_skips_the_probe() {
        let c = resolve(EngineKind::Lle, None, || panic!("must not probe"));
        assert_eq!(c.requested, EngineKind::Lle);
        assert_eq!(c.effective, EngineKind::Hle);
        let n = c.notice.unwrap();
        assert!(n.contains("Nord OS update file") && n.contains("ns4 firmware set"));
        assert!(n.contains("HLE"));
    }

    #[test]
    fn lle_with_the_stub_backend_falls_back_with_the_reason() {
        let c = resolve(
            EngineKind::Lle,
            Some(Path::new("os.zip")),
            LleBackend::probe,
        );
        assert_eq!(c.effective, EngineKind::Hle);
        assert!(c.notice.unwrap().contains("not implemented"));
    }

    #[test]
    fn lle_runs_when_file_and_backend_are_there() {
        let c = resolve(EngineKind::Lle, Some(Path::new("os.zip")), || {
            LleAvailability::Available
        });
        assert_eq!(c.effective, EngineKind::Lle);
        assert_eq!(c.notice, None);
    }

    #[test]
    fn the_flag_beats_the_environment_beats_the_saved_value() {
        let r = |a: &[&str], e, sv| requested(&s(a), e, sv);
        assert_eq!(r(&[], None, None), Ok(EngineKind::Hle));
        assert_eq!(r(&[], None, Some("lle")), Ok(EngineKind::Lle));
        assert_eq!(r(&[], Some("hle"), Some("lle")), Ok(EngineKind::Hle));
        assert_eq!(
            r(&["--engine", "lle"], Some("hle"), None),
            Ok(EngineKind::Lle)
        );
        assert_eq!(r(&[], Some(" "), Some("lle")), Ok(EngineKind::Lle));
    }

    #[test]
    fn bad_values_are_errors_that_name_their_source() {
        assert!(from_args(&s(&["ns4", "--engine"]))
            .unwrap_err()
            .contains("needs a value"));
        assert!(from_args(&s(&["ns4", "--engine", "x"])).is_err());
        assert!(requested(&[], Some("x"), None)
            .unwrap_err()
            .contains(ENGINE_ENV));
        assert!(requested(&[], None, Some("x"))
            .unwrap_err()
            .contains("saved engine"));
    }

    #[test]
    fn a_saved_choice_round_trips_and_keeps_the_update_path() {
        let cfg = std::env::temp_dir()
            .join(format!("ns4-engine-{}", std::process::id()))
            .join("config.json");
        crate::user_config::update(&cfg, |c| c.os_update_path = Some("/os.zip".into())).unwrap();
        save_choice(&cfg, EngineKind::Lle).unwrap();
        let u = crate::user_config::load(&cfg).unwrap();
        assert_eq!(
            requested(&[], None, u.engine.as_deref()),
            Ok(EngineKind::Lle)
        );
        assert_eq!(u.os_update_path.as_deref(), Some("/os.zip"));
    }

    #[test]
    fn startup_with_a_bad_flag_runs_hle_and_never_panics() {
        let c = startup_with(
            "test",
            &s(&["x", "--engine", "bogus"]),
            None,
            None,
            None,
            None,
        );
        assert_eq!(c.effective, EngineKind::Hle);
        assert!(c.notice.unwrap().contains("bogus"));
    }

    #[test]
    fn startup_with_saved_lle_and_no_file_prompts_and_a_corrupt_config_is_not_silent() {
        let c = startup_with("test", &[], None, Some("lle"), None, None);
        assert_eq!(
            (c.requested, c.effective),
            (EngineKind::Lle, EngineKind::Hle)
        );
        assert!(c.notice.unwrap().contains("Nord OS update file"));
        // a corrupt config is reported on stderr and the choice stays HLE
        let c = startup_with("test", &[], None, None, Some("bad json"), None);
        assert_eq!(c.effective, EngineKind::Hle);
    }

    #[test]
    fn the_update_file_must_exist_to_count_as_set() {
        assert_eq!(os_update_from(None), None);
        assert_eq!(os_update_from(Some("  ")), None);
        assert_eq!(os_update_from(Some("/definitely/not/here.zip")), None);
        let here = std::env::temp_dir();
        assert_eq!(os_update_from(Some(here.to_str().unwrap())), Some(here));
    }

    #[test]
    fn strip_flag_removes_every_engine_pair_wherever_it_sits() {
        assert_eq!(
            strip_flag(&s(&["ns4", "--engine", "lle", "info"])),
            s(&["ns4", "info"])
        );
        assert_eq!(
            strip_flag(&s(&["ns4", "info", "--engine", "hle", "--x"])),
            s(&["ns4", "info", "--x"])
        );
        assert_eq!(strip_flag(&s(&["ns4", "--engine"])), s(&["ns4"]));
    }
}
