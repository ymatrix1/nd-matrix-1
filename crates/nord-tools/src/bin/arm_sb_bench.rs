//! OWNER: W1-CNT. Deterministic counters of the ARM0 boot and phases for the tier decisions
//! (gate G0 of `docs/research/arm0-jit-design.md`). Skeleton: the argument parser, the counter
//! row and the table printer; the measurement itself is not written yet and `main` says so and
//! exits non-zero (it never prints numbers it did not measure).
//!
//!     cargo run --release -p nord-tools --bin arm-sb-bench -- \
//!         <bank0_CODE.bin> <bank0_SRAM.bin> <eeprom_image_256k.bin> [--golden <digest file>]
//!
//! Environment (as `arm-lle-bench`): `TICK_INSNS` (instructions per 1 ms tick, default 200000),
//! `PHASE_SCALE` (percent of the tick counts, default 100).
//!
//! Spec (WP W1-CNT): per phase (load, idle, one key, eight keys) report `probe_executed /
//! executed`, the fixed tick cost split (tick time minus executed instructions x 5.3 ns), the
//! executed instructions per tick at the 1 ms and the 24 kHz framing, and compare the state
//! hash and message digest with the golden file; exit non-zero on a mismatch. Uses the main
//! APIs only (`Machine`, `IdleStats`), so it runs on `main` and on this branch alike.

// Skeleton: the counters, statistics and printers are consumed by `main` once W1-CNT lands;
// until then only the tests use them.
#![allow(dead_code)]

use nord_arm::machine::idle::IdleStats;
use std::path::PathBuf;

const USAGE: &str = "usage: arm-sb-bench <bank0_CODE.bin> <bank0_SRAM.bin> <eeprom_image_256k.bin> [--golden <file>]";

/// ASSUMED: ARM instructions per 1 ms tick, as `arm-lle-bench` (`tools/arm_steady.py`
/// `INSNS_PER_TICK`); `TICK_INSNS` overrides it. Confirmed only by a delay-loop reference.
const TICK_DEFAULT: u64 = 200_000;

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    code: PathBuf,
    sram: PathBuf,
    eeprom: PathBuf,
    tick_insns: u64,
    phase_scale: u64,
    golden: Option<PathBuf>,
}

/// Parse the arguments after the program name; `env` reads an environment variable.
fn parse_args(
    args: impl IntoIterator<Item = String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Args, String> {
    let (mut files, mut golden) = (Vec::new(), None);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--golden" => golden = Some(PathBuf::from(it.next().ok_or("--golden needs a file")?)),
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            _ => files.push(PathBuf::from(a)),
        }
    }
    let [code, sram, eeprom]: [PathBuf; 3] = files
        .try_into()
        .map_err(|_| format!("expected three input files\n{USAGE}"))?;
    let number = |key: &str, default: u64| -> Result<u64, String> {
        match env(key) {
            None => Ok(default),
            Some(v) => v.parse().map_err(|_| format!("{key}={v} is not a number")),
        }
    };
    let (tick_insns, phase_scale) = (
        number("TICK_INSNS", TICK_DEFAULT)?,
        number("PHASE_SCALE", 100)?,
    );
    if tick_insns == 0 || phase_scale == 0 {
        return Err("TICK_INSNS and PHASE_SCALE must be positive".into());
    }
    Ok(Args {
        code,
        sram,
        eeprom,
        tick_insns,
        phase_scale,
        golden,
    })
}

/// Counters of one phase.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    name: String,
    ticks: u64,
    executed: u64,
    probe_executed: u64,
}

impl Row {
    fn from_stats(name: &str, ticks: u64, s: &IdleStats) -> Self {
        Self {
            name: name.into(),
            ticks,
            executed: s.executed,
            probe_executed: s.probe_executed,
        }
    }

    /// `probe_executed / executed` in percent (0 when nothing executed).
    fn probe_share(&self) -> f64 {
        if self.executed == 0 {
            0.0
        } else {
            100.0 * self.probe_executed as f64 / self.executed as f64
        }
    }

    /// Executed instructions per tick of `hz` framing, for a 1 ms tick of `tick_insns`
    /// instructions (a 24 kHz frame is 1/24 of the 1 ms budget).
    ///
    /// ASSUMED: the DSP frame rate is 24 kHz (design note, gate G0 item c); what would confirm it
    /// is the `arm-lle-bench` message cadence in the `eight keys` phase.
    fn per_tick(&self, hz: u64) -> f64 {
        if self.ticks == 0 {
            0.0
        } else {
            self.executed as f64 / self.ticks as f64 * 1000.0 / hz as f64
        }
    }
}

/// The report table, one line per phase.
fn table(rows: &[Row]) -> String {
    let mut out = format!(
        "{:<12} {:>8} {:>14} {:>14} {:>9} {:>12} {:>12}\n",
        "phase", "ticks", "executed", "probe_exec", "probe %", "insn/1kHz", "insn/24kHz"
    );
    for r in rows {
        out += &format!(
            "{:<12} {:>8} {:>14} {:>14} {:>8.1}% {:>12.0} {:>12.0}\n",
            r.name,
            r.ticks,
            r.executed,
            r.probe_executed,
            r.probe_share(),
            r.per_tick(1000),
            r.per_tick(24_000)
        );
    }
    out
}

/// Whether `got` equals the golden digest line (surrounding whitespace ignored).
fn digest_matches(golden: &str, got: &str) -> bool {
    golden.trim() == got.trim()
}

fn main() {
    let args = match parse_args(std::env::args().skip(1), &|k| std::env::var(k).ok()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "arm-sb-bench: counters are not implemented yet (OWNER W1-CNT); nothing was measured for {}",
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

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn three_files_and_the_defaults_parse() {
        let a = parse_args(args(&["c.bin", "s.bin", "e.bin"]), &no_env).unwrap();
        assert_eq!(
            (
                a.code,
                a.sram,
                a.eeprom,
                a.tick_insns,
                a.phase_scale,
                a.golden
            ),
            (
                "c.bin".into(),
                "s.bin".into(),
                "e.bin".into(),
                200_000,
                100,
                None
            )
        );
    }

    #[test]
    fn the_environment_and_the_golden_option_are_read() {
        let env = |k: &str| (k == "TICK_INSNS").then(|| "50000".to_string());
        let a = parse_args(args(&["--golden", "g.txt", "c", "s", "e"]), &env).unwrap();
        assert_eq!((a.tick_insns, a.golden), (50_000, Some("g.txt".into())));
    }

    #[test]
    fn bad_arguments_are_rejected_with_the_usage() {
        for bad in [
            &["c", "s"][..],
            &["c", "s", "e", "x"],
            &["--nope", "c", "s", "e"],
            &["--golden"],
        ] {
            assert!(parse_args(args(bad), &no_env).is_err(), "{bad:?}");
        }
        let env = |_: &str| Some("abc".to_string());
        assert!(parse_args(args(&["c", "s", "e"]), &env)
            .unwrap_err()
            .contains("not a number"));
        let zero = |_: &str| Some("0".to_string());
        assert!(parse_args(args(&["c", "s", "e"]), &zero).is_err());
        assert!(parse_args(args(&["-h"]), &no_env)
            .unwrap_err()
            .contains("usage"));
    }

    #[test]
    fn the_rows_compute_the_probe_share_and_the_per_tick_framing() {
        let stats = IdleStats {
            executed: 4_800_000,
            probe_executed: 1_200_000,
            ..IdleStats::default()
        };
        let r = Row::from_stats("idle", 24, &stats);
        assert_eq!(r.probe_share(), 25.0);
        assert_eq!(r.per_tick(1000), 200_000.0);
        assert_eq!(r.per_tick(24_000), 200_000.0 / 24.0);
        let empty = Row::from_stats("none", 0, &IdleStats::default());
        assert_eq!((empty.probe_share(), empty.per_tick(1000)), (0.0, 0.0));
    }

    #[test]
    fn the_table_has_a_header_and_one_line_per_phase() {
        let stats = IdleStats {
            executed: 1000,
            probe_executed: 500,
            ..IdleStats::default()
        };
        let t = table(&[
            Row::from_stats("load", 10, &stats),
            Row::from_stats("idle", 10, &stats),
        ]);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("phase") && lines[1].starts_with("load"));
        assert!(lines[1].contains("50.0%"), "{}", lines[1]);
    }

    #[test]
    fn a_digest_mismatch_is_detected_whatever_the_whitespace() {
        assert!(digest_matches("abc123\n", "  abc123"));
        assert!(!digest_matches("abc123", "abc124"));
    }
}
