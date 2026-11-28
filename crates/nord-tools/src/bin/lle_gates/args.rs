//! Command line of `lle-gates`.

use crate::load::LOAD_LIMIT_PCT;
use std::path::PathBuf;

pub const USAGE: &str = "usage: lle-gates [--fw DIR] [--bank N] [--synthetic] [--only sharc|arm]\n\
  [--engines interp,aot,uop,uop-checked,uop-native] [--tiers plain,predecode,sb,sb-native]\n\
  [--runs N] [--passes N] [--warmup N] [--tick-insns N] [--phase-scale PCT]\n\
  [--org-image F] [--pno-image F] [--eeprom F] [--aot-tag-org T] [--aot-tag-pno T]\n\
  [--uop-ranges extent|probe] [--load-limit PCT] [--load-window-ms N] [--indicative]\n\
  [--json FILE] [--uop-reports FILE]\n\
see docs/research/lle-gates-howto.md";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Only {
    Both,
    Sharc,
    Arm,
}

#[derive(Debug, Clone)]
pub struct Args {
    /// Directory with the user's own images (`.reference/fw_unpacked` by default; never committed).
    pub fw: PathBuf,
    pub bank: u32,
    /// Measure the synthetic poll core instead of firmware (no files needed).
    pub synthetic: bool,
    pub only: Only,
    pub engines: Vec<String>,
    pub tiers: Vec<String>,
    /// Measured rounds after one discarded warm-up round.
    pub runs: usize,
    pub passes: u32,
    pub warmup: u32,
    pub tick_insns: u64,
    pub phase_scale: usize,
    pub org_image: Option<PathBuf>,
    pub pno_image: Option<PathBuf>,
    pub eeprom: Option<PathBuf>,
    pub aot_tag_org: Option<String>,
    pub aot_tag_pno: Option<String>,
    pub ranges: String,
    pub load_limit: f64,
    pub load_window_ms: u64,
    /// Print timings even when the host load invalidates them, marked as indicative.
    pub indicative: bool,
    pub json: Option<PathBuf>,
    pub uop_reports: Option<PathBuf>,
}

/// ASSUMED: seven measured rounds, as the A/B protocol of `arm_sb_ab.rs` (one more is discarded).
pub const RUNS_DEFAULT: usize = 7;

fn list(v: &str) -> Vec<String> {
    v.split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut a = Args {
        fw: PathBuf::from(".reference/fw_unpacked"),
        bank: 0,
        synthetic: false,
        only: Only::Both,
        engines: list("interp,aot,uop,uop-checked,uop-native"),
        tiers: list("plain,predecode,sb,sb-native"),
        runs: RUNS_DEFAULT,
        passes: 300,
        warmup: 16,
        tick_insns: 200_000,
        phase_scale: 100,
        org_image: None,
        pno_image: None,
        eeprom: None,
        aot_tag_org: None,
        aot_tag_pno: None,
        ranges: "extent".into(),
        load_limit: LOAD_LIMIT_PCT,
        load_window_ms: 250,
        indicative: false,
        json: None,
        uop_reports: None,
    };
    let mut it = args.into_iter();
    while let Some(k) = it.next() {
        if k == "-h" || k == "--help" {
            return Err(USAGE.into());
        }
        if k == "--synthetic" || k == "--indicative" {
            *(if k == "--synthetic" {
                &mut a.synthetic
            } else {
                &mut a.indicative
            }) = true;
            continue;
        }
        let v = it.next().ok_or(format!("{k} needs a value\n{USAGE}"))?;
        let num = |what: &str| v.parse::<u64>().map_err(|e| format!("{what}: {e}"));
        match k.as_str() {
            "--fw" => a.fw = v.into(),
            "--bank" => a.bank = num("--bank")? as u32,
            "--only" => {
                a.only = match v.as_str() {
                    "sharc" => Only::Sharc,
                    "arm" => Only::Arm,
                    _ => return Err(format!("--only {v:?}: want sharc or arm")),
                }
            }
            "--engines" => a.engines = list(&v),
            "--tiers" => a.tiers = list(&v),
            "--runs" => a.runs = num("--runs")? as usize,
            "--passes" => a.passes = num("--passes")? as u32,
            "--warmup" => a.warmup = num("--warmup")? as u32,
            "--tick-insns" => a.tick_insns = num("--tick-insns")?,
            "--phase-scale" => a.phase_scale = num("--phase-scale")? as usize,
            "--org-image" => a.org_image = Some(v.into()),
            "--pno-image" => a.pno_image = Some(v.into()),
            "--eeprom" => a.eeprom = Some(v.into()),
            "--aot-tag-org" => a.aot_tag_org = Some(v),
            "--aot-tag-pno" => a.aot_tag_pno = Some(v),
            "--uop-ranges" => a.ranges = v,
            "--load-limit" => a.load_limit = v.parse().map_err(|e| format!("--load-limit: {e}"))?,
            "--load-window-ms" => a.load_window_ms = num("--load-window-ms")?,
            "--json" => a.json = Some(v.into()),
            "--uop-reports" => a.uop_reports = Some(v.into()),
            _ => return Err(format!("unknown option {k}\n{USAGE}")),
        }
    }
    if a.runs == 0 || a.passes == 0 || a.phase_scale == 0 {
        return Err("--runs, --passes and --phase-scale must be positive".into());
    }
    for e in &a.engines {
        if crate::sharc::ENGINES.iter().all(|x| x.name != e) {
            return Err(format!("--engines {e:?}: unknown engine"));
        }
    }
    for t in &a.tiers {
        if crate::arm::TIERS.iter().all(|x| x.0 != t) {
            return Err(format!("--tiers {t:?}: unknown tier"));
        }
    }
    Ok(a)
}

impl Args {
    pub fn org_path(&self) -> PathBuf {
        self.org_image
            .clone()
            .unwrap_or_else(|| self.fw.join("ORG/core2_sram.bin"))
    }
    pub fn pno_path(&self) -> PathBuf {
        self.pno_image
            .clone()
            .unwrap_or_else(|| self.fw.join("PNO/core2_sram.bin"))
    }
    /// `(code, sram, eeprom)` of the ARM0 image of the chosen bank.
    pub fn arm_paths(&self) -> [PathBuf; 3] {
        [
            self.fw.join(format!("bank{}_CODE.bin", self.bank)),
            self.fw.join(format!("bank{}_SRAM.bin", self.bank)),
            self.eeprom
                .clone()
                .unwrap_or_else(|| self.fw.join("eeprom_image_256k.bin")),
        ]
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_follow_the_ab_protocol_and_the_reference_layout() {
        let a = parse_args(args(&[])).unwrap();
        assert_eq!(
            (a.runs, a.passes, a.warmup, a.load_limit),
            (7, 300, 16, 25.0)
        );
        assert_eq!(
            a.org_path(),
            PathBuf::from(".reference/fw_unpacked/ORG/core2_sram.bin")
        );
        let [code, sram, ee] = a.arm_paths();
        assert!(code.ends_with("bank0_CODE.bin") && sram.ends_with("bank0_SRAM.bin"));
        assert!(ee.ends_with("eeprom_image_256k.bin"));
        assert_eq!(a.engines.len(), 5);
        assert_eq!(a.tiers.len(), 4);
    }

    #[test]
    fn options_override_the_layout() {
        let a = parse_args(args(&[
            "--fw",
            "/x",
            "--bank",
            "1",
            "--org-image",
            "o.bin",
            "--synthetic",
            "--only",
            "sharc",
            "--engines",
            "uop,interp",
            "--runs",
            "3",
            "--indicative",
            "--load-limit",
            "10",
        ]))
        .unwrap();
        assert_eq!(a.org_path(), PathBuf::from("o.bin"));
        assert_eq!(a.pno_path(), PathBuf::from("/x/PNO/core2_sram.bin"));
        assert!(a.arm_paths()[0].ends_with("bank1_CODE.bin"));
        assert!(a.synthetic && a.indicative && a.only == Only::Sharc);
        assert_eq!((a.runs, a.load_limit), (3, 10.0));
        assert_eq!(a.engines, ["uop", "interp"]);
    }

    #[test]
    fn bad_command_lines_are_rejected_with_a_reason() {
        for bad in [
            &["--bogus", "1"][..],
            &["--runs", "0"],
            &["--runs"],
            &["--engines", "turbo"],
            &["--tiers", "t9"],
            &["--only", "both"],
            &["--passes", "x"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
        assert!(parse_args(args(&["--help"]))
            .unwrap_err()
            .starts_with("usage"));
    }
}
