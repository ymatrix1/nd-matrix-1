//! `sharc-live IMAGE [--pno|--org|--sy|--smp] [--epdma] [--epchain] [--isrs N] [--ring-dma] [--probe PC]... [--lane SET:PAIR]
//! [--window ADDR:COUNT:STRIDE]`: one DSP
//! chip as a line-protocol server (D138). The ARM harness (`tools/arm_dsp_link.py`) writes the
//! messages the firmware sends (`M W1 W2`), asks for passes (`P N`) and reads the summary lines;
//! the protocol is in `nord_sharc::runner::live`. Prints `READY` once the chip has booted.
//! Evidence: `docs/research/arm-dsp-live-link.md`.
use nord_sharc::runner::live::{boot_core, preset_named, Live};
use nord_sharc::runner::live_sy::SyModel;
use nord_sharc::runner::parse_number;
use nord_sharc::runner::pass::PassConfig;
use nord_sharc::runner::periph_run::PeriphOpts;
use std::io::{BufRead, Write};
use std::process::ExitCode;

/// Level window `(address, count, stride)`: float words read after every pass (a synth voice
/// ends in a channel record, not in a lane).
type Window = Option<(u32, u32, u32)>;

/// Parsed command line.
struct Parsed {
    image: String,
    periph: PeriphOpts,
    probes: Vec<u32>,
    lane: (u32, u32),
    window: Window,
    preset: &'static str,
    /// Audio frames per pass of the SY chip (`--isrs N`, default 2, D153).
    isrs: u32,
}

fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut p = Parsed {
        image: String::new(),
        periph: PeriphOpts::default(),
        probes: vec![],
        lane: (2, 0),
        window: None,
        preset: "pno",
        isrs: 2,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pno" => p.preset = "pno",
            "--org" => p.preset = "org", // ORG core 1/2 and PNO core 2 (organ program, D144)
            "--sy" => p.preset = "sy",   // SY1SY2 sample chip, interrupt-driven pass (D153)
            "--smp" => p.preset = "smp", // SMP piano chip, same pass model, ring tap (OQ41)
            "--window" => {
                let s = it.next().ok_or("--window needs ADDR:COUNT:STRIDE")?;
                let bad = || format!("bad --window {s:?} (want ADDR:COUNT:STRIDE)");
                let f: Vec<u32> = s
                    .split(':')
                    .map(parse_number)
                    .collect::<Option<_>>()
                    .ok_or_else(bad)?;
                match f[..] {
                    [a, n, st] if n > 0 && st > 0 && n <= 64 => p.window = Some((a, n, st)),
                    _ => return Err(bad()),
                }
            }
            "--isrs" => {
                p.isrs = it
                    .next()
                    .and_then(|s| parse_number(s))
                    .filter(|n| (1..=64).contains(n))
                    .ok_or("--isrs needs 1..64")?;
            }
            "--probe" => p.probes.push(
                it.next()
                    .and_then(|s| parse_number(s))
                    .ok_or("bad --probe")?,
            ),
            "--lane" => {
                let s = it.next().ok_or("--lane needs SET:PAIR")?;
                let bad = || format!("bad --lane {s:?} (want SET:PAIR)");
                let (a, b) = s.split_once(':').ok_or_else(bad)?;
                p.lane = (
                    parse_number(a).ok_or_else(bad)?,
                    parse_number(b).ok_or_else(bad)?,
                );
                if p.lane.0 > 5 || p.lane.1 > 2 {
                    return Err(bad());
                }
            }
            _ if p.periph.take_flag(a) => {}
            _ if p.image.is_empty() && !a.starts_with("--") => p.image = a.clone(),
            _ => return Err(format!("unknown argument {a:?}")),
        }
    }
    if p.image.is_empty() {
        return Err("usage: sharc-live IMAGE [--pno|--org|--sy|--smp] [--epdma] [--epchain] [--isrs N] [--ring-dma] [--probe PC]... [--lane SET:PAIR] [--window ADDR:COUNT:STRIDE]".into());
    }
    Ok(p)
}

fn serve(args: &[String]) -> Result<(), String> {
    let a = parse(args)?;
    let data = std::fs::read(&a.image).map_err(|e| format!("{}: {e}", a.image))?;
    let (periph, p) = (a.periph, preset_named(a.preset).ok_or("unknown preset")?);
    let core = boot_core(&data, &p, &periph)?;
    let cfg = PassConfig {
        trigger: p.trigger,
        idle_pc: p.idle_pc,
        max_steps: 200_000,
        irq: periph.start_status(p.irq),
    };
    let mut live = Live::new(core, cfg, p.ping_pong, a.lane, a.probes);
    if p.smp {
        live = live.with_sy(SyModel::smp(a.isrs));
    } else if p.sy {
        live = live.with_sy(SyModel::new(a.isrs));
    }
    if let Some((addr, n, st)) = a.window {
        live = live.with_window(addr, n, st);
    }
    let (stdin, stdout) = (std::io::stdin(), std::io::stdout());
    let mut out = stdout.lock();
    writeln!(out, "READY").map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim() == "Q" {
            break;
        }
        if let Some(reply) = live.handle(&line) {
            writeln!(out, "{reply}").map_err(|e| e.to_string())?;
        }
        if line.trim_start().starts_with(['P', 'D']) {
            out.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match serve(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sharc-live: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn arguments_select_image_probes_lane_and_models() {
        let a = parse(&args(&[
            "core1.bin",
            "--pno",
            "--epdma",
            "--probe",
            "0x1256AD",
            "--probe",
            "7",
            "--lane",
            "1:2",
        ]))
        .unwrap();
        assert_eq!(
            (a.image.as_str(), a.probes, a.lane, a.preset),
            ("core1.bin", vec![0x1256AD, 7], (1, 2), "pno")
        );
        assert!(a.periph.epdma && !a.periph.ring_dma);
        assert_eq!(
            parse(&args(&["a.bin"])).unwrap().lane,
            (2, 0),
            "default lane 2:0"
        );
    }

    #[test]
    fn org_flag_and_voice_window_are_parsed() {
        let a = parse(&args(&["o.bin", "--org", "--window", "0xC1F83:4:2"])).unwrap();
        assert_eq!((a.preset, a.window), ("org", Some((0xC_1F83, 4, 2))));
        assert_eq!(parse(&args(&["o.bin"])).unwrap().window, None);
    }

    #[test]
    fn sy_flag_and_frames_per_pass_are_parsed() {
        let a = parse(&args(&["sy.bin", "--sy", "--isrs", "1", "--epchain"])).unwrap();
        assert_eq!((a.preset, a.isrs, a.periph.epchain), ("sy", 1, true));
        assert_eq!(parse(&args(&["sy.bin", "--sy"])).unwrap().isrs, 2);
        assert_eq!(parse(&args(&["smp.bin", "--smp"])).unwrap().preset, "smp");
    }

    #[test]
    fn bad_arguments_are_refused() {
        for bad in [
            &["a.bin", "--lane", "9:0"][..],
            &["a.bin", "--bogus"],
            &[],
            &["a", "--probe", "zz"],
            &["a", "--window", "1:2"],
            &["a", "--window", "1:zz:2:3"],
            &["a", "--window", "1:2:2:3"],
            &["a", "--window", "1:0:2"],
            &["a", "--window", "1:99:2"],
            &["a", "--isrs", "0"],
            &["a", "--isrs", "65"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
