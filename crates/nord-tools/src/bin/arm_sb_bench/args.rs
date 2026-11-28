//! Command line and environment of `arm-sb-bench` (arguments as `arm-lle-bench`).

use std::path::PathBuf;

pub const USAGE: &str = "usage: arm-sb-bench <bank0_CODE.bin> <bank0_SRAM.bin> <eeprom_image_256k.bin> [--golden <file>] [--time]\n\
environment: TICK_INSNS (virtual instructions per 1 ms tick, default 200000), PHASE_SCALE (percent of the phase lengths, default 100), PREDECODE=0 (plain Cpu::step path)";

/// ASSUMED: ARM instructions per 1 ms tick, as `arm-lle-bench` (`tools/arm_steady.py`
/// `INSNS_PER_TICK`); `TICK_INSNS` overrides it. What would confirm it: a firmware delay loop of
/// known wall-clock duration (design note follow-up F1).
pub const TICK_DEFAULT: u64 = 200_000;

/// ASSUMED: the OS timer interrupt is 24 kHz, so a 24 kHz tick is 1/24 of a 1 ms tick
/// (`docs/research/arm-tick-timers.md`, D130; the 200 k instructions per ms it divides is itself
/// ASSUMED, see [`TICK_DEFAULT`]). Gate G0 (c) measures the executed instructions per tick at this
/// framing instead of dividing the 1 ms figure by it.
pub const FRAME_DIVISOR: u64 = 24;

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub code: PathBuf,
    pub sram: PathBuf,
    pub eeprom: PathBuf,
    pub tick_insns: u64,
    pub phase_scale: u64,
    /// `PREDECODE=0` runs the plain `Cpu::step` path (the oracle tier).
    pub predecode: bool,
    pub golden: Option<PathBuf>,
    /// `--time`: also record thread CPU time per tick (non-deterministic, off by default).
    pub time: bool,
}

/// Parse the arguments after the program name; `env` reads an environment variable.
pub fn parse_args(
    args: impl IntoIterator<Item = String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Args, String> {
    let (mut files, mut golden, mut time) = (Vec::new(), None, false);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--golden" => golden = Some(PathBuf::from(it.next().ok_or("--golden needs a file")?)),
            "--time" => time = true,
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
    if tick_insns < FRAME_DIVISOR || phase_scale == 0 {
        return Err(format!(
            "TICK_INSNS must be at least {FRAME_DIVISOR} and PHASE_SCALE positive"
        ));
    }
    Ok(Args {
        code,
        sram,
        eeprom,
        tick_insns,
        phase_scale,
        predecode: env("PREDECODE").is_none_or(|v| v != "0"),
        golden,
        time,
    })
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
            a,
            Args {
                code: "c.bin".into(),
                sram: "s.bin".into(),
                eeprom: "e.bin".into(),
                tick_insns: 200_000,
                phase_scale: 100,
                predecode: true,
                golden: None,
                time: false,
            }
        );
    }

    #[test]
    fn options_and_the_environment_are_read_in_any_order() {
        let env = |k: &str| match k {
            "TICK_INSNS" => Some("50000".to_string()),
            "PHASE_SCALE" => Some("7".to_string()),
            "PREDECODE" => Some("0".to_string()),
            _ => None,
        };
        let a = parse_args(args(&["--golden", "g.txt", "c", "--time", "s", "e"]), &env).unwrap();
        assert_eq!(
            (a.tick_insns, a.phase_scale, a.predecode, a.golden, a.time),
            (50_000, 7, false, Some("g.txt".into()), true)
        );
        // any PREDECODE value but "0" keeps the cache on, as `arm-lle-bench`
        let on = |k: &str| (k == "PREDECODE").then(|| "1".to_string());
        assert!(parse_args(args(&["c", "s", "e"]), &on).unwrap().predecode);
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
        let words = |_: &str| Some("abc".to_string());
        assert!(parse_args(args(&["c", "s", "e"]), &words)
            .unwrap_err()
            .contains("not a number"));
        let zero = |_: &str| Some("0".to_string());
        assert!(parse_args(args(&["c", "s", "e"]), &zero).is_err());
        let help = parse_args(args(&["-h"]), &no_env).unwrap_err();
        assert!(help.contains("usage") && help.contains("--time"), "{help}");
    }

    #[test]
    fn a_tick_too_short_for_the_24khz_framing_is_rejected() {
        let env = |k: &str| (k == "TICK_INSNS").then(|| (FRAME_DIVISOR - 1).to_string());
        assert!(parse_args(args(&["c", "s", "e"]), &env).is_err());
        let ok = |k: &str| (k == "TICK_INSNS").then(|| FRAME_DIVISOR.to_string());
        assert_eq!(
            parse_args(args(&["c", "s", "e"]), &ok).unwrap().tick_insns,
            FRAME_DIVISOR
        );
    }
}
