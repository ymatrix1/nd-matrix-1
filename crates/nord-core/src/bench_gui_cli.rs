//! Command line of `ns4 bench --gui` (see [`crate::bench_gui`]).

use crate::bench_gui::{breakdown_with, format_row, run_with, GuiBenchConfig};
use crate::render_opts::FirmwareOptions;

fn num<T: std::str::FromStr>(args: &[String], flag: &str) -> Result<Option<T>, String> {
    match args.iter().position(|a| a == flag) {
        None => Ok(None),
        Some(i) => args
            .get(i + 1)
            .and_then(|v| v.parse().ok())
            .map(Some)
            .ok_or_else(|| format!("{flag} needs a number")),
    }
}

/// Parses the flags of `ns4 bench --gui`. Firmware flags go through [`FirmwareOptions::from_args`],
/// so no flag means the GUI's start-up options.
pub fn parse(args: &[String]) -> Result<GuiBenchConfig, String> {
    let mut cfg = GuiBenchConfig {
        options: FirmwareOptions::from_args(args)?,
        ..GuiBenchConfig::default()
    };
    cfg.layers = num(args, "--layers")?.unwrap_or(cfg.layers);
    if cfg.layers != 3 && cfg.layers != 7 {
        return Err("--layers is 3 (Scene I) or 7 (Scene II)".into());
    }
    cfg.keys = num(args, "--keys")?.unwrap_or(cfg.keys).min(88);
    cfg.frames = num(args, "--frames")?.unwrap_or(cfg.frames).max(1);
    cfg.blocks = num(args, "--blocks")?.unwrap_or(cfg.blocks);
    cfg.warmup_blocks = num(args, "--warmup")?.unwrap_or(cfg.warmup_blocks);
    cfg.churn_every = num(args, "--churn")?.unwrap_or(cfg.churn_every);
    cfg.repeats = num(args, "--repeat")?.unwrap_or(cfg.repeats).max(1);
    Ok(cfg)
}

/// Runs the bench and returns the report text.
pub fn cli(args: &[String]) -> Result<String, String> {
    let cfg = parse(args)?;
    let source = crate::bench_gui_prog::parse(args)?;
    let base = |e: &mut crate::engine::Stage4Engine| {
        if let Some(s) = &source {
            if let Err(err) = s.apply(&cfg, e) {
                panic!("program setup failed: {err}");
            }
        }
    };
    let mut probe = crate::bench_gui::engine(&cfg);
    base(&mut probe);
    let fx = crate::bench_gui_prog::fx_summary(&probe);
    let r = run_with(cfg, base);
    let mut out = format!(
        "ns4 bench --gui: {} layers, {} keys, {} frames @ 48000 Hz (deadline {:.3} ms), churn {}, options {}\nFX racks:\n{}\n{}\nhash {:#018x} (peak {:.3}, master clamp hit {} of {} frames)",
        cfg.layers,
        cfg.keys,
        cfg.frames,
        r.deadline_ms,
        cfg.churn_every,
        cfg.options.summary(),
        fx,
        format_row("render_frame path", &r),
        r.hash,
        r.peak,
        r.clamped,
        r.blocks * cfg.frames
    );
    if cfg.churn_every > 0 {
        out.push_str(&format!(
            "\nkey-press blocks: mean {:.3} max {:.3} ms; other blocks: mean {:.3} ms",
            r.press_mean_ms, r.press_max_ms, r.quiet_mean_ms
        ));
    }
    if let Some(loads) = num::<usize>(args, "--reload")? {
        let src = source
            .as_ref()
            .filter(|s| !s.backup.is_empty())
            .ok_or("--reload needs --backup and --program")?;
        let second = args
            .iter()
            .position(|a| a == "--program2")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        let programs = crate::bench_gui_reload::decode_programs(
            src,
            second,
            num::<usize>(args, "--reload-pool")?.unwrap_or(0),
        )?;
        let rep = crate::bench_gui_reload::run(&cfg, src, &programs, loads)?;
        out.push('\n');
        out.push_str(&rep.format());
    }
    if args.iter().any(|a| a == "--breakdown") {
        out.push_str("\nbreakdown (ms per block):");
        for (label, row) in breakdown_with(cfg, &base) {
            out.push_str(&format!("\n  {}", format_row(&label, &row)));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_parse_and_default_to_the_gui_options() {
        let c = parse(&a(&["bench", "--gui"])).unwrap();
        assert_eq!((c.layers, c.keys, c.frames), (7, 88, 128));
        assert_eq!(c.options, FirmwareOptions::default());
        let c = parse(&a(&[
            "bench", "--gui", "--layers", "3", "--keys", "20", "--frames", "512", "--churn", "8",
            "--warmup", "5",
        ]))
        .unwrap();
        assert_eq!((c.layers, c.keys, c.frames, c.churn_every), (3, 20, 512, 8));
        assert_eq!(c.warmup_blocks, 5);
    }

    #[test]
    fn bad_values_are_rejected() {
        assert!(parse(&a(&["bench", "--layers", "5"])).is_err());
        assert!(parse(&a(&["bench", "--keys", "many"])).is_err());
        assert!(parse(&a(&["bench", "--tone-source", "nope"])).is_err());
    }

    #[test]
    fn cli_prints_a_row_hash_and_the_breakdown() {
        let t = cli(&a(&[
            "bench",
            "--gui",
            "--layers",
            "3",
            "--keys",
            "4",
            "--blocks",
            "6",
            "--churn",
            "2",
            "--breakdown",
        ]))
        .unwrap();
        assert!(
            t.contains("render_frame path") && t.contains("hash 0x"),
            "{t}"
        );
        assert!(
            t.contains("key-press blocks") && t.contains("Organ A only"),
            "{t}"
        );
    }
}
