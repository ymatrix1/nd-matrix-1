//! OWNER: S0 (frozen). Switches and tuning constants of the ARM0 execution tiers.
//!
//! Everything defaults to off: `SbConfig::default()` changes no behaviour. Runtime switches (no
//! other feature flag exists): `NS4_SB=1` (tier 2), `NS4_SB_IDLE2=1` (idle skipping v2),
//! `NS4_SB_NATIVE=1` (tier 3, needs the `sb-native` cargo feature), `NS4_SB_HOT=n`,
//! `NS4_SB_VALIDATE=n`.
//!
//! ASSUMED (all numeric defaults): chosen from the 7.8-instruction block length and the 2 KB hot
//! routine of the heavy stretch (design note section 3.3); the first counters run (gate G0)
//! fixes them.

/// Tier switches and trace-formation limits.
#[derive(Clone, Debug)]
pub struct SbConfig {
    /// Tier 2: run hot code as trace superblocks.
    pub enabled: bool,
    /// Idle skipping v2 (`machine/sb/idle.rs`, `loopwatch.rs`).
    pub idle_v2: bool,
    /// Tier 3: compile traces to native code (feature `sb-native`).
    pub native: bool,
    /// Dispatches of a pc before a trace is built there.
    pub hot: u16,
    /// Instructions per tier-1 stretch between dispatch points.
    pub slice: u32,
    pub max_uops: usize,
    pub max_guards: usize,
    pub max_pages: usize,
    pub inline_depth: u8,
    /// Traces built per `run()` call at most.
    pub builds_per_run: u8,
    /// Uops in the cache before it is flushed.
    pub uop_cap: usize,
    /// A page whose code words really changed this many times becomes tier-1 only.
    pub volatile_after: u8,
    /// Random states each new trace is validated on (0 = off).
    pub validate_cases: u32,
    /// Peephole passes (`fuse.rs`, `pred.rs`, `flagopt.rs`) and literal folding; only consulted
    /// when the tier is enabled.
    pub fuse: bool,
    pub predicate: bool,
    pub dead_flags: bool,
    pub fold_literals: bool,
}

impl Default for SbConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_v2: false,
            native: false,
            hot: 24,
            slice: 32,
            max_uops: 128,
            max_guards: 6,
            max_pages: 4,
            inline_depth: 2,
            builds_per_run: 4,
            uop_cap: 4_000_000,
            volatile_after: 4,
            validate_cases: 0,
            fuse: false,
            predicate: false,
            dead_flags: false,
            fold_literals: false,
        }
    }
}

impl SbConfig {
    /// Tier 2 on with every optimisation and `hot = 0` (a trace is built at the first
    /// dispatch), for tests and tools.
    pub fn on() -> Self {
        Self {
            enabled: true,
            hot: 0,
            fuse: true,
            predicate: true,
            dead_flags: true,
            fold_literals: true,
            ..Self::default()
        }
    }

    /// The configuration of the process environment (see the module docs); unset or malformed
    /// variables leave the default.
    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// [`SbConfig::from_env`] over any variable source (tests pass a table; the process
    /// environment is shared between test threads).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let flag = |k: &str| get(k).is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let mut c = if flag("NS4_SB") {
            // the tier's optimisations follow the tier switch (there is no flag of their own)
            Self {
                hot: Self::default().hot,
                ..Self::on()
            }
        } else {
            Self::default()
        };
        c.idle_v2 = flag("NS4_SB_IDLE2");
        c.native = flag("NS4_SB_NATIVE");
        if let Some(v) = get("NS4_SB_HOT").and_then(|v| v.trim().parse().ok()) {
            c.hot = v;
        }
        if let Some(v) = get("NS4_SB_VALIDATE").and_then(|v| v.trim().parse().ok()) {
            c.validate_cases = v;
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> SbConfig {
        SbConfig::from_lookup(|k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        })
    }

    #[test]
    fn the_default_changes_no_behaviour_and_keeps_the_documented_constants() {
        let c = SbConfig::default();
        assert!(!c.enabled && !c.idle_v2 && !c.native);
        assert!(!c.fuse && !c.predicate && !c.dead_flags && !c.fold_literals);
        assert_eq!(c.validate_cases, 0);
        assert_eq!((c.hot, c.slice, c.max_uops, c.max_guards), (24, 32, 128, 6));
        assert_eq!((c.max_pages, c.inline_depth, c.builds_per_run), (4, 2, 4));
        assert_eq!((c.uop_cap, c.volatile_after), (4_000_000, 4));
    }

    #[test]
    fn on_builds_at_the_first_dispatch_and_keeps_native_and_idle_off() {
        let c = SbConfig::on();
        assert!(
            c.enabled && c.hot == 0 && c.fuse && c.predicate && c.dead_flags && c.fold_literals
        );
        assert!(!c.idle_v2 && !c.native);
        assert_eq!(c.slice, SbConfig::default().slice);
    }

    #[test]
    fn an_empty_environment_is_the_default() {
        let c = env(&[]);
        assert!(!c.enabled && !c.idle_v2 && !c.native && c.hot == 24 && !c.fuse);
    }

    #[test]
    fn the_switches_map_to_their_fields_and_malformed_values_are_ignored() {
        let c = env(&[
            ("NS4_SB", "1"),
            ("NS4_SB_IDLE2", "true"),
            ("NS4_SB_NATIVE", "1"),
            ("NS4_SB_HOT", "7"),
            ("NS4_SB_VALIDATE", "8"),
        ]);
        assert!(c.enabled && c.idle_v2 && c.native && c.fuse);
        assert_eq!((c.hot, c.validate_cases), (7, 8));
        let c = env(&[("NS4_SB", "1")]);
        assert_eq!(c.hot, 24, "NS4_SB alone keeps the default hotness");
        let c = env(&[
            ("NS4_SB", "0"),
            ("NS4_SB_HOT", "x"),
            ("NS4_SB_VALIDATE", "-1"),
        ]);
        assert!(!c.enabled && c.hot == 24 && c.validate_cases == 0);
    }
}
