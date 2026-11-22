//! Level census of the synth layers over the factory programs for opt-in synth flags
//! (`synth_census`, `docs/research/synth-rows-unison.md`). Offline tooling: allocates freely.
//!
//! Each variant is a named list of `FirmwareOptions` CLI flags rendered against the engine
//! default; the census reports the RMS level change of the mono mean of one note per program
//! (tool convention, the same for every variant, so only differences carry meaning).

use nord_core::render_opts::FirmwareOptions;

/// One census variant: label and parsed options.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub opts: FirmwareOptions,
}

/// Parses `NAME=FLAG FLAG ...` (the flags are whitespace separated `FirmwareOptions` arguments;
/// `NAME=` alone is the engine default).
pub fn parse_variant(spec: &str) -> Result<Variant, String> {
    let (name, flags) = spec
        .split_once('=')
        .ok_or_else(|| format!("variant {spec:?}: expected NAME=FLAGS"))?;
    if name.trim().is_empty() {
        return Err(format!("variant {spec:?}: empty name"));
    }
    let args: Vec<String> = flags.split_whitespace().map(str::to_owned).collect();
    let opts = FirmwareOptions::from_args(&args)?;
    Ok(Variant {
        name: name.trim().to_owned(),
        opts,
    })
}

/// Distribution of a set of level changes in dB.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub median_db: f64,
    pub mean_abs_db: f64,
    pub p10_db: f64,
    pub p90_db: f64,
    pub within_3_db: usize,
}

/// Summary of `deltas` (NaN entries are dropped); `None` when nothing is left.
pub fn summarize(deltas: &[f64]) -> Option<Summary> {
    let mut v: Vec<f64> = deltas.iter().copied().filter(|d| d.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    Some(Summary {
        n: v.len(),
        median_db: at(0.5),
        mean_abs_db: v.iter().map(|d| d.abs()).sum::<f64>() / v.len() as f64,
        p10_db: at(0.1),
        p90_db: at(0.9),
        within_3_db: v.iter().filter(|d| d.abs() <= 3.0).count(),
    })
}

/// `to - from` in dB, `NaN` when either render is silent (floor of `note_metrics`).
pub fn level_delta_db(from_db: f64, to_db: f64) -> f64 {
    if from_db <= -150.0 || to_db <= -150.0 {
        f64::NAN
    } else {
        to_db - from_db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_parse_flags_and_reject_bad_specs() {
        let v = parse_variant("uw=--synth-unison-weights").unwrap();
        assert_eq!(v.name, "uw");
        assert!(v.opts.synth_unison_weights);
        let d = parse_variant("base=").unwrap();
        assert_eq!(d.opts, FirmwareOptions::default());
        assert!(parse_variant("noequals").is_err());
        assert!(parse_variant("=--synth-stage3").is_err());
        assert!(parse_variant("x=--synth-unison-gain nope").is_err());
    }

    #[test]
    fn summary_reports_median_spread_and_the_three_db_count() {
        let s = summarize(&[-6.0, -1.0, 0.0, 2.0, f64::NAN, 4.0]).unwrap();
        assert_eq!(s.n, 5);
        assert_eq!(s.median_db, 0.0);
        assert_eq!(s.within_3_db, 3);
        assert!((s.mean_abs_db - 2.6).abs() < 1e-12);
        assert_eq!((s.p10_db, s.p90_db), (-6.0, 4.0));
        assert!(summarize(&[f64::NAN]).is_none());
    }

    #[test]
    fn silent_renders_have_no_level_delta() {
        assert_eq!(level_delta_db(-20.0, -17.0), 3.0);
        assert!(level_delta_db(-200.0, -17.0).is_nan());
        assert!(level_delta_db(-20.0, -200.0).is_nan());
    }
}
