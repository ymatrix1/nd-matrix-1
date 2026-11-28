//! OWNER: W4-BENCH. The single A/B of gates G2 and G3 of `docs/research/arm0-jit-design.md`:
//! tiers off against tiers on, interleaved, thread CPU cycles, host load recorded. Skeleton: the
//! statistics, the verdict rule and the printer; the runs themselves are not written yet and
//! `main` says so and exits non-zero (it never reports a verdict it did not measure).
//!
//!     cargo run --release -p nord-tools --bin arm-sb-ab -- \
//!         <bank0_CODE.bin> <bank0_SRAM.bin> <eeprom_image_256k.bin> [--runs 7]
//!
//! Protocol (the repository's): 7 interleaved runs per arm, one warm-up discarded, median
//! [min..max]; a gain counts only when the min..max ranges do not overlap; the host
//! `LoadPercentage` is sampled before each run and a verdict is valid only below
//! [`LOAD_LIMIT_PCT`], otherwise it is printed as `indicative`. An unknown load is not below
//! the limit.

// Skeleton: the counters, statistics and printers are consumed by `main` once W4-BENCH lands;
// until then only the tests use them.
#![allow(dead_code)]

use std::path::PathBuf;

const USAGE: &str =
    "usage: arm-sb-ab <bank0_CODE.bin> <bank0_SRAM.bin> <eeprom_image_256k.bin> [--runs N]";

/// Highest host load, in percent, at which a verdict is reported as valid (the A/B protocol of
/// `docs/research/arm0-jit-design.md`, section 12; the SHARC spike host sat at 77 to 100 %).
const LOAD_LIMIT_PCT: f64 = 25.0;

/// Runs per arm (the protocol's 7, one more is discarded as warm-up).
const RUNS_DEFAULT: usize = 7;

#[derive(Debug, PartialEq, Eq)]
struct Args {
    code: PathBuf,
    sram: PathBuf,
    eeprom: PathBuf,
    runs: usize,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let (mut files, mut runs) = (Vec::new(), RUNS_DEFAULT);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--runs" => {
                runs = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|n| *n >= 1)
                    .ok_or("--runs needs a positive number")?;
            }
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            _ => files.push(PathBuf::from(a)),
        }
    }
    let [code, sram, eeprom]: [PathBuf; 3] = files
        .try_into()
        .map_err(|_| format!("expected three input files\n{USAGE}"))?;
    Ok(Args {
        code,
        sram,
        eeprom,
        runs,
    })
}

/// Median of `xs` (mean of the two middle values for an even count); `None` when empty.
fn median(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// `(min, max)` of `xs`; `None` when empty.
fn range(xs: &[f64]) -> Option<(f64, f64)> {
    let min = xs.iter().copied().min_by(f64::total_cmp)?;
    let max = xs.iter().copied().max_by(f64::total_cmp)?;
    Some((min, max))
}

/// What the A/B says about arm B (the tiers) against arm A (tiers off); lower is better.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    /// B is faster and the ranges do not overlap, on a host below the load limit.
    Faster,
    /// B is slower and the ranges do not overlap, on a host below the load limit.
    Slower,
    /// The ranges overlap: no difference can be claimed.
    Inconclusive,
    /// The host load was above the limit or unknown: numbers only, no verdict.
    Indicative,
}

/// The verdict of two samples taken at the host load `load_pct` (the highest sampled value,
/// `None` when it could not be read).
fn verdict(a: &[f64], b: &[f64], load_pct: Option<f64>) -> Verdict {
    if !matches!(load_pct, Some(l) if l < LOAD_LIMIT_PCT) {
        return Verdict::Indicative;
    }
    match (range(a), range(b)) {
        (Some((amin, _)), Some((_, bmax))) if bmax < amin => Verdict::Faster,
        (Some((_, amax)), Some((bmin, _))) if bmin > amax => Verdict::Slower,
        _ => Verdict::Inconclusive,
    }
}

/// One report line: `median [min..max]` of a sample in `unit`.
fn fmt_sample(name: &str, xs: &[f64], unit: &str) -> String {
    match (median(xs), range(xs)) {
        (Some(m), Some((lo, hi))) => format!("{name:<10} {m:>12.1} [{lo:.1}..{hi:.1}] {unit}"),
        _ => format!("{name:<10} no samples"),
    }
}

/// The A/B report: both samples and the verdict (with the load that qualified it).
fn report(a: &[f64], b: &[f64], load_pct: Option<f64>, unit: &str) -> String {
    let load = load_pct.map_or("unknown".to_string(), |l| format!("{l:.0} %"));
    let v = verdict(a, b, load_pct);
    let word = match v {
        Verdict::Faster => "faster (ranges do not overlap)",
        Verdict::Slower => "slower (ranges do not overlap)",
        Verdict::Inconclusive => "inconclusive (ranges overlap)",
        Verdict::Indicative => "indicative (host load above the limit or unknown)",
    };
    format!(
        "{}\n{}\nhost load {load} (limit {LOAD_LIMIT_PCT:.0} %): {word}\n",
        fmt_sample("tiers off", a, unit),
        fmt_sample("tiers on", b, unit)
    )
}

fn main() {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "arm-sb-ab: the A/B is not implemented yet (OWNER W4-BENCH); {} runs per arm were not made for {}",
        args.runs,
        args.code.display()
    );
    std::process::exit(3);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_files_and_the_run_count_parse_and_bad_input_is_rejected() {
        let a = parse_args(args(&["c", "s", "e"])).unwrap();
        assert_eq!((a.runs, a.code), (7, "c".into()));
        assert_eq!(
            parse_args(args(&["c", "s", "e", "--runs", "3"]))
                .unwrap()
                .runs,
            3
        );
        for bad in [
            &["c", "s"][..],
            &["c", "s", "e", "--runs", "0"],
            &["c", "s", "e", "--runs"],
            &["--x", "c", "s", "e"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn median_and_range_handle_odd_even_and_empty_samples() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!((median(&[]), range(&[])), (None, None));
        assert_eq!(range(&[3.0, 1.0, 2.0]), Some((1.0, 3.0)));
    }

    #[test]
    fn a_gain_needs_disjoint_ranges_and_a_quiet_host() {
        let (off, on) = ([100.0, 102.0, 101.0], [50.0, 52.0, 51.0]);
        assert_eq!(verdict(&off, &on, Some(10.0)), Verdict::Faster);
        assert_eq!(verdict(&on, &off, Some(10.0)), Verdict::Slower);
        assert_eq!(
            verdict(&off, &[99.0, 120.0], Some(10.0)),
            Verdict::Inconclusive,
            "overlap"
        );
        assert_eq!(
            verdict(&off, &on, Some(25.0)),
            Verdict::Indicative,
            "the limit itself is not below it"
        );
        assert_eq!(verdict(&off, &on, Some(77.0)), Verdict::Indicative);
        assert_eq!(
            verdict(&off, &on, None),
            Verdict::Indicative,
            "unknown load"
        );
        assert_eq!(verdict(&[], &on, Some(1.0)), Verdict::Inconclusive);
    }

    #[test]
    fn the_report_prints_both_samples_and_never_a_verdict_on_a_loaded_host() {
        let (off, on) = ([100.0, 102.0, 101.0], [50.0, 52.0, 51.0]);
        let quiet = report(&off, &on, Some(8.0), "ms");
        assert!(
            quiet.contains("faster") && quiet.contains("101.0 [100.0..102.0] ms"),
            "{quiet}"
        );
        let loaded = report(&off, &on, Some(77.0), "ms");
        assert!(
            loaded.contains("indicative") && !loaded.contains("faster"),
            "{loaded}"
        );
        assert!(report(&[], &[], None, "ms").contains("no samples"));
    }

    #[test]
    fn the_load_limit_is_the_protocols_25_percent() {
        assert_eq!(LOAD_LIMIT_PCT, 25.0);
    }
}
