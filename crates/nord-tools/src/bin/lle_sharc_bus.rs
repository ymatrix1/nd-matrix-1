//! `lle_sharc_bus`: several SHARC chips on the inter-chip audio bus (bean `ns4-lle4`), run offline
//! tick by tick on one thread (deterministic, not real time, `docs/research/sharc-lle-realtime.md`).
//! Frames of the tone chips go into the FX cores and the last stage into a host stereo WAV, no
//! intermediate WAV hand-over.
//!
//! ```text
//! lle_sharc_bus --node NAME=IMAGE,PRESET[,MSGFILE]... --route FROM>TO[:GAIN]... [--in-wav NODE:FILE]...
//!               [--ticks N] [--wav OUT]
//!   PRESET  org | pno (2 frames out per tick), sy | smp (2 audio interrupts), fx1 | fx2 (4 frames in,
//!           4 out, every 2nd tick), see `runner::lle_system`
//!   TO      a node name, or `host` (the output buffer)
//!   --in-wav  host input: the frames of a stereo WAV are queued in front of NODE (an FX core)
//!   MSGFILE ARM load script (`PASS W1 W2` lines, all delivered before pass 0) or `--msg` tokens
//! ```
//! Example (local images only): `--node org=ORG/core2_sram.bin,org,org.script
//! --node fx2=FX/core2_sram.bin,fx2,fx2.script --route org>fx2 --route fx2>host --ticks 4000`.
//! Prints per node the passes and the frames short at the input, and the host peak / rms.
use nord_sharc::runner::audio::wav_bytes;
use nord_sharc::runner::fx_run::read_wav;
use nord_sharc::runner::lle_system::{build_bus, build_node, NodeSpec};
use nord_sharc::runner::pass_sched::Engine;
use nord_sharc::runner::system::FRAME_RATE;
use std::fmt::Write as _;

struct Args {
    nodes: Vec<NodeSpec>,
    routes: Vec<String>,
    ticks: u32,
    wav: Option<String>,
    in_wavs: Vec<(String, String)>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        nodes: Vec::new(),
        routes: Vec::new(),
        ticks: 1000,
        wav: None,
        in_wavs: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(k) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{k} needs a value"));
        match k.as_str() {
            "--node" => a.nodes.push(NodeSpec::parse(&val()?)?),
            "--route" => a.routes.push(val()?),
            "--ticks" => a.ticks = val()?.parse().map_err(|e| format!("--ticks: {e}"))?,
            "--wav" => a.wav = Some(val()?),
            "--in-wav" => {
                let v = val()?;
                let (n, f) = v.split_once(':').ok_or("--in-wav needs NODE:FILE")?;
                a.in_wavs.push((n.into(), f.into()));
            }
            _ => return Err(format!("unknown argument {k:?}")),
        }
    }
    if a.nodes.is_empty() {
        return Err("usage: lle_sharc_bus --node NAME=IMAGE,PRESET[,MSGFILE]... --route FROM>TO[:GAIN]... [--in-wav NODE:FILE]... [--ticks N] [--wav OUT]".into());
    }
    Ok(a)
}

/// `(peak, rms)` of both channels, finite samples only.
fn level(l: &[f32], r: &[f32]) -> (f32, f64) {
    let v: Vec<f32> = l
        .iter()
        .chain(r)
        .copied()
        .filter(|x| x.is_finite())
        .collect();
    let peak = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    let ss: f64 = v.iter().map(|x| f64::from(*x).powi(2)).sum();
    (peak, (ss / v.len().max(1) as f64).sqrt())
}

fn run(args: &[String]) -> Result<String, String> {
    let a = parse(args)?;
    let mut nodes = Vec::new();
    for s in &a.nodes {
        let data = std::fs::read(&s.image).map_err(|e| format!("{}: {e}", s.image))?;
        let msgs = match &s.msgs {
            Some(f) => std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?,
            None => String::new(),
        };
        nodes.push(build_node(s, &data, &msgs, Engine::Interp)?);
    }
    let names: Vec<String> = a.nodes.iter().map(|s| s.name.clone()).collect();
    let mut bus = build_bus(&names, nodes, &a.routes)?;
    for (n, f) in &a.in_wavs {
        let i = names
            .iter()
            .position(|x| x == n)
            .ok_or(format!("--in-wav: no node named {n:?}"))?;
        let bytes = std::fs::read(f).map_err(|e| format!("{f}: {e}"))?;
        let (ch, _) = read_wav(&bytes)?;
        let r = ch.get(1).unwrap_or(&ch[0]);
        let frames: Vec<[f32; 2]> = ch[0].iter().zip(r).map(|(l, r)| [*l, *r]).collect();
        bus.push_input(i, &frames);
    }
    bus.run(a.ticks)?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} ticks, {} host frames",
        bus.tick,
        bus.host.left.len()
    );
    for (n, u) in names.iter().zip(&bus.underruns) {
        let _ = writeln!(out, "{n}: input frames short {u}");
    }
    let (peak, rms) = level(&bus.host.left, &bus.host.right);
    let _ = writeln!(out, "host peak {peak:.6} rms {rms:.6}");
    if let Some(f) = &a.wav {
        let w = wav_bytes(
            &[bus.host.left.clone(), bus.host.right.clone()],
            FRAME_RATE as u32,
        );
        std::fs::write(f, w).map_err(|e| format!("{f}: {e}"))?;
        let _ = writeln!(out, "wrote {f}");
    }
    Ok(out)
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    print!("{}", run(&args)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn arguments_parse_and_reject_the_unknown() {
        let a = parse(&v(&[
            "--node",
            "o=a.bin,org",
            "--route",
            "o>host",
            "--ticks",
            "5",
            "--wav",
            "x.wav",
        ]))
        .unwrap();
        assert_eq!((a.nodes.len(), a.routes.len(), a.ticks), (1, 1, 5));
        assert_eq!(a.wav.as_deref(), Some("x.wav"));
        assert!(parse(&v(&[])).is_err(), "no node is a usage error");
        assert!(parse(&v(&["--node", "o=a.bin,org", "--bogus"])).is_err());
        assert!(parse(&v(&["--node", "o=a.bin,org", "--ticks", "x"])).is_err());
        assert!(parse(&v(&["--node"])).is_err());
        let a = parse(&v(&["--node", "o=a.bin,org", "--in-wav", "o:in.wav"])).unwrap();
        assert_eq!(a.in_wavs, vec![("o".to_string(), "in.wav".to_string())]);
        assert!(parse(&v(&["--node", "o=a.bin,org", "--in-wav", "nocolon"])).is_err());
    }

    #[test]
    fn level_ignores_non_finite_samples_and_run_reports_a_missing_image() {
        let (p, r) = level(&[0.5, f32::NAN], &[-1.0, 0.0]);
        assert_eq!(p, 1.0);
        assert!((r - (1.25f64 / 3.0).sqrt()).abs() < 1e-9);
        assert_eq!(level(&[], &[]), (0.0, 0.0));
        let e = run(&v(&["--node", "o=/nonexistent/x.bin,org"])).unwrap_err();
        assert!(e.contains("x.bin"), "{e}");
    }
}
