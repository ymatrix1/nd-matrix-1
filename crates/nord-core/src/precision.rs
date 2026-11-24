//! SHARC float precision of the opt-in LLE engine (bean `ns4-lle7`, D246; epic wording in PR #662).
//!
//! `Exact` emulates the 40-bit extended-precision float of the SHARC bit-for-bit and is the only
//! mode used as the oracle. `Fast` runs the same lowered code with native `f32` for real-time play;
//! it is audibly equivalent within a tolerance that the LLE benchmarks (`ns4-lle3`) still have to
//! measure, and is never used for oracle comparison. The LLE engine does not exist yet: the value is
//! only plumbed to the backend seam (`engine_select::EngineChoice::precision`). It matters only when
//! the LLE engine is selected; HLE ignores it.
//!
//! Precedence of the request, first match wins: `--precision exact|fast`, then `NS4_PRECISION`, then
//! the saved `precision` key of the user config, then `Exact`. A bad value never stops start-up: the
//! default is used and the notice names the bad value.

use std::path::Path;

/// Environment variable that selects the precision (below the flag, above the saved value).
pub const PRECISION_ENV: &str = "NS4_PRECISION";

/// How the LLE engine computes SHARC floating point.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PrecisionMode {
    /// Emulated 40-bit extended-precision float, bit-exact (default, oracle).
    #[default]
    Exact,
    /// Native `f32`: real-time play, audibly equivalent within a measured tolerance (never the oracle).
    Fast,
}

impl PrecisionMode {
    /// Lower-case name used on the command line, in the environment and in the config file.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Fast => "fast",
        }
    }

    /// Parses `exact` or `fast` (case-insensitive); anything else is a plain-language error.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "exact" => Ok(Self::Exact),
            "fast" => Ok(Self::Fast),
            other => Err(format!("unknown precision \"{other}\" (use exact or fast)")),
        }
    }
}

/// The value after `--precision` in `args`: `Ok(None)` when absent, `Err` when missing or unknown.
pub fn from_args(args: &[String]) -> Result<Option<PrecisionMode>, String> {
    let Some(i) = args.iter().position(|a| a == "--precision") else {
        return Ok(None);
    };
    match args.get(i + 1) {
        Some(v) => PrecisionMode::parse(v).map(Some),
        None => Err("--precision needs a value (exact or fast)".into()),
    }
}

/// The requested precision from the flag, the environment value and the saved value (module doc
/// for the order). An unknown value at the winning level is an error naming its source.
pub fn requested(
    args: &[String],
    env: Option<&str>,
    saved: Option<&str>,
) -> Result<PrecisionMode, String> {
    if let Some(m) = from_args(args)? {
        return Ok(m);
    }
    for (v, what) in [(env, PRECISION_ENV), (saved, "saved precision setting")] {
        if let Some(v) = v.map(str::trim).filter(|v| !v.is_empty()) {
            return PrecisionMode::parse(v).map_err(|e| format!("{what}: {e}"));
        }
    }
    Ok(PrecisionMode::Exact)
}

/// [`requested`], never failing: a bad value gives `Exact` plus a notice for the user.
pub fn resolve(
    args: &[String],
    env: Option<&str>,
    saved: Option<&str>,
) -> (PrecisionMode, Option<String>) {
    match requested(args, env, saved) {
        Ok(m) => (m, None),
        Err(e) => (
            PrecisionMode::Exact,
            Some(format!("{e}. Using exact (bit-exact) precision instead.")),
        ),
    }
}

/// `args` without every `--precision <value>` pair (see `engine_select::strip_flag`).
pub fn strip_flag(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--precision" {
            it.next();
        } else {
            out.push(a.clone());
        }
    }
    out
}

/// Binaries, once at start-up after `engine_select::startup`: reads the flag, environment and saved
/// value and returns `choice` with its `precision` set. A bad value prints its notice on stderr as
/// `<who>: <notice>` and `Exact` is used. An unreadable config was already reported by `startup`.
pub fn apply_startup(
    who: &str,
    args: &[String],
    choice: crate::engine_select::EngineChoice,
) -> crate::engine_select::EngineChoice {
    let saved = crate::user_config::default_config_path()
        .and_then(|p| crate::user_config::load(&p).ok())
        .and_then(|c| c.precision);
    let env = std::env::var(PRECISION_ENV).ok();
    apply_with(who, args, env.as_deref(), saved.as_deref(), choice)
}

/// [`apply_startup`] with every input given (no process state), for tests.
pub fn apply_with(
    who: &str,
    args: &[String],
    env: Option<&str>,
    saved: Option<&str>,
    mut choice: crate::engine_select::EngineChoice,
) -> crate::engine_select::EngineChoice {
    let (mode, notice) = resolve(args, env, saved);
    if let Some(n) = notice {
        eprintln!("{who}: {n}");
    }
    choice.precision = mode;
    choice
}

/// Remembers `mode` in the config file at `config` (other saved settings are kept).
pub fn save_choice(config: &Path, mode: PrecisionMode) -> Result<(), String> {
    crate::user_config::update(config, |c| c.precision = Some(mode.as_str().into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn names_parse_and_default_is_exact() {
        assert_eq!(PrecisionMode::parse(" FAST "), Ok(PrecisionMode::Fast));
        assert_eq!(PrecisionMode::parse("exact"), Ok(PrecisionMode::Exact));
        assert_eq!(PrecisionMode::Fast.as_str(), "fast");
        assert_eq!(PrecisionMode::default(), PrecisionMode::Exact);
        assert!(PrecisionMode::parse("f64").unwrap_err().contains("f64"));
    }

    #[test]
    fn the_flag_beats_the_environment_beats_the_saved_value() {
        let r = |a: &[&str], e, sv| requested(&s(a), e, sv);
        assert_eq!(r(&[], None, None), Ok(PrecisionMode::Exact));
        assert_eq!(r(&[], None, Some("fast")), Ok(PrecisionMode::Fast));
        assert_eq!(
            r(&[], Some("exact"), Some("fast")),
            Ok(PrecisionMode::Exact)
        );
        assert_eq!(
            r(&["--precision", "fast"], Some("exact"), Some("exact")),
            Ok(PrecisionMode::Fast)
        );
        assert_eq!(r(&[], Some(" "), Some("fast")), Ok(PrecisionMode::Fast));
    }

    #[test]
    fn a_bad_value_falls_back_to_exact_with_a_notice_naming_the_source() {
        let (m, n) = resolve(&s(&["x", "--precision", "half"]), None, None);
        assert_eq!(m, PrecisionMode::Exact);
        assert!(n.unwrap().contains("half"));
        let (_, n) = resolve(&[], Some("bad"), None);
        assert!(n.unwrap().contains(PRECISION_ENV));
        let (_, n) = resolve(&[], None, Some("bad"));
        assert!(n.unwrap().contains("saved precision"));
        let (_, n) = resolve(&s(&["x", "--precision"]), None, None);
        assert!(n.unwrap().contains("needs a value"));
        assert_eq!(resolve(&[], None, None).1, None);
    }

    #[test]
    fn apply_with_plumbs_the_value_to_the_engine_choice() {
        let base = crate::engine_select::resolve(
            crate::engine_select::EngineKind::Lle,
            None,
            crate::engine_select::LleBackend::probe,
        );
        assert_eq!(base.precision, PrecisionMode::Exact);
        let c = apply_with("t", &s(&["--precision", "fast"]), None, None, base.clone());
        assert_eq!(c.precision, PrecisionMode::Fast);
        // the engine decision is untouched, and a bad value does not stop start-up
        assert_eq!((c.requested, c.effective), (base.requested, base.effective));
        assert_eq!(
            apply_with("t", &[], Some("bad"), None, base).precision,
            PrecisionMode::Exact
        );
    }

    #[test]
    fn strip_flag_removes_every_precision_pair() {
        assert_eq!(
            strip_flag(&s(&["ns4", "--precision", "fast", "info", "--precision"])),
            s(&["ns4", "info"])
        );
    }

    #[test]
    fn a_saved_choice_round_trips_and_an_old_config_loads_unchanged() {
        let dir = std::env::temp_dir().join(format!("ns4-prec-{}", std::process::id()));
        let cfg = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        // a config written before this key existed: no `precision`, no `engine`
        std::fs::write(&cfg, r#"{ "os_update_path": "/os.zip" }"#).unwrap();
        let old = crate::user_config::load(&cfg).unwrap();
        assert_eq!(
            (old.os_update_path.as_deref(), old.precision),
            (Some("/os.zip"), None)
        );
        save_choice(&cfg, PrecisionMode::Fast).unwrap();
        let u = crate::user_config::load(&cfg).unwrap();
        assert_eq!(u.os_update_path.as_deref(), Some("/os.zip"));
        assert_eq!(
            requested(&[], None, u.precision.as_deref()),
            Ok(PrecisionMode::Fast)
        );
    }
}
