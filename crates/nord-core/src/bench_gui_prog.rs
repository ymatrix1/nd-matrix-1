//! Program loading for `ns4 bench --gui --backup <.ns4b> --program <entry>`: the bench plays a real
//! factory program (with its sample banks) instead of the start-up engine, optionally with the
//! arpeggiator forced on, so the timing covers the layers the owner actually plays.

use crate::bench_gui::GuiBenchConfig;
use crate::engine::Stage4Engine;
use crate::synth_types::ArpMode;

/// Where the program comes from and what to force on after loading.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProgramSource {
    /// `.ns4b` backup archive.
    pub backup: String,
    /// Entry name or unique substring inside the archive (e.g. `Horizon Whl`).
    pub entry: String,
    /// Force this arpeggiator mode on Synth A..C (`--arp up|down|updown|random|poly|gate`).
    pub arp: Option<ArpMode>,
}

/// Parses an `--arp` value.
pub fn parse_arp(v: &str) -> Result<ArpMode, String> {
    match v {
        "up" => Ok(ArpMode::Up),
        "down" => Ok(ArpMode::Down),
        "updown" => Ok(ArpMode::UpDown),
        "random" => Ok(ArpMode::Random),
        "poly" => Ok(ArpMode::Poly),
        "gate" => Ok(ArpMode::Gate),
        _ => Err(format!(
            "--arp '{v}' (expected up|down|updown|random|poly|gate)"
        )),
    }
}

/// Reads `--backup`, `--program` and `--arp` (both of the first two, or neither).
pub fn parse(args: &[String]) -> Result<Option<ProgramSource>, String> {
    let flag = |n: &str| {
        args.iter()
            .position(|a| a == n)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let arp = flag("--arp").map(|v| parse_arp(&v)).transpose()?;
    match (flag("--backup"), flag("--program")) {
        (Some(backup), Some(entry)) => Ok(Some(ProgramSource { backup, entry, arp })),
        (None, None) if arp.is_none() => Ok(None),
        (None, None) => Ok(Some(ProgramSource {
            arp,
            ..ProgramSource::default()
        })),
        _ => Err("--backup and --program go together".into()),
    }
}

impl ProgramSource {
    /// Loads the program (when named), re-applies the firmware options (as `render-patch` does
    /// after loading) and forces the arpeggiator mode. Setup cost, never on the audio thread.
    pub fn apply(&self, cfg: &GuiBenchConfig, e: &mut Stage4Engine) -> Result<(), String> {
        if !self.backup.is_empty() {
            let archive =
                crate::cbin::Ns4BackupArchive::open(&self.backup).map_err(|x| x.to_string())?;
            let mut file = std::fs::File::open(&self.backup).map_err(|x| x.to_string())?;
            e.load_program_with_sounds_from_backup(&archive, &mut file, &self.entry)
                .map_err(|x| x.to_string())?;
            e.scene.scene_2_active = cfg.layers > 3;
            cfg.options.apply_exact(e);
        }
        if let Some(mode) = self.arp {
            for s in [&mut e.synth_a, &mut e.synth_b, &mut e.synth_c] {
                s.arp_mode = mode;
            }
        }
        Ok(())
    }
}

/// One line per layer FX chain: what each rack runs (reverb model and mix, Mod 1 / Mod 2 types,
/// delay, compressor), so a slow run can be read against the program's effects.
pub fn fx_summary(e: &Stage4Engine) -> String {
    let names = [
        "Organ A", "Organ B", "Piano A", "Piano B", "Synth A", "Synth B", "Synth C",
    ];
    e.fx_chains
        .iter()
        .zip(names)
        .map(|(c, n)| {
            format!(
                "  {n}: reverb {:?} mix {:.2}, mod1 {:?}, mod2 {:?}, delay {}, comp {}",
                c.reverb_model,
                c.reverb_mix,
                c.mod1_type,
                c.mod2_type,
                if c.delay_enabled { "on" } else { "off" },
                if c.comp_enabled { "on" } else { "off" },
            )
        })
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_parse_in_pairs_and_arp_alone() {
        assert_eq!(parse(&a(&["bench"])).unwrap(), None);
        let p = parse(&a(&[
            "--backup",
            "x.ns4b",
            "--program",
            "Horizon",
            "--arp",
            "up",
        ]));
        let p = p.unwrap().unwrap();
        assert_eq!((p.backup.as_str(), p.entry.as_str()), ("x.ns4b", "Horizon"));
        assert_eq!(p.arp, Some(ArpMode::Up));
        assert!(parse(&a(&["--backup", "x"])).is_err());
        assert!(parse(&a(&["--arp", "sideways"])).is_err());
        assert_eq!(parse(&a(&["--arp", "gate"])).unwrap().unwrap().backup, "");
    }

    #[test]
    fn arp_only_source_forces_the_mode_on_all_synth_layers() {
        let cfg = GuiBenchConfig::default();
        let src = parse(&a(&["--arp", "random"])).unwrap().unwrap();
        let mut e = crate::bench_gui::engine(&cfg);
        src.apply(&cfg, &mut e).unwrap();
        assert_eq!(e.synth_b.arp_mode, ArpMode::Random);
        let t = fx_summary(&e);
        assert_eq!(t.lines().count(), 7);
        assert!(t.contains("Synth C: reverb"), "{t}");
        let bad = ProgramSource {
            backup: "no-such.ns4b".into(),
            entry: "x".into(),
            arp: None,
        };
        assert!(bad.apply(&cfg, &mut e).is_err());
    }

    #[test]
    fn idle_skip_keeps_the_hash_of_a_scene() {
        use crate::bench_gui::run_with;
        // Scene I: Organ B, Piano B and Synth B..C never get a note, so their racks fall silent
        // for good and sleep after 4 s (420 blocks of 480 frames = 4.2 s); the audible layers run
        // as before, so the output is bit-identical.
        let cfg = GuiBenchConfig {
            layers: 3,
            keys: 3,
            frames: 480,
            blocks: 420,
            ..GuiBenchConfig::default()
        };
        let plain = run_with(cfg, |_| {});
        let skipped = run_with(cfg, |e| {
            for c in e.fx_chains.iter_mut() {
                c.set_idle_skip(true);
            }
        });
        assert_eq!(plain.hash, skipped.hash);
        assert!(plain.peak > 0.0);
    }
}
