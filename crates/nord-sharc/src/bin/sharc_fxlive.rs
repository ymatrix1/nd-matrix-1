//! `sharc-fxlive IMAGE [options]`: the FX chip Core2 pass on live audio (D122). The ARM message
//! scripts are delivered before pass 0, a stereo signal is written into the pass input block
//! (4 frames per pass), the pass runs in the interpreter and the 4 output frames of every pass are
//! collected.
//!
//! ```text
//! --core 1|2               FX core (default 2; core 1 is the one with the PNO input path)
//! --passes N               passes to run (default 2000)         --boot N (default 3000000)
//! --script-flat FILE       every message of FILE before pass 0 (the ARM boot log of the FX chip)
//! --msg [P:]W1=W2          extra message before pass P (default 0): DM word W1 = W2, W2 may be f1.5
//! --slot ADDR              input block (8 words) to fill; repeat for several (default 0xC0180)
//! --in-wav FILE            stereo or mono WAV as input (a PNO lane WAV from sharc-sys --lane --wav)
//! --sine HZ:AMP            sine input instead (default 440:0.25); --impulse AMP: one frame
//! --lead N                 N passes of silence before the source (the master stage fades in over
//!                          about 2000 passes, so an impulse at pass 0 is lost)
//! --tap ADDR              also record the 8-word block at ADDR after every pass (a bus, for example
//!                          0xC0458 on core 1); with --wav the tap goes to FILE.tap.wav
//! --rate R                 frame rate of generated input and the output WAV (default 43402)
//! --wav FILE               write the stereo output as a float WAV
//! --calls-at PC            after the run: one more pass, print how often PC is reached (stage calls)
//! --regs-at PC             after the run: one more pass, print the M and I registers where PC is
//!                          first executed (the entry registers of a stage routine)
//! --dump ADDR:COUNT        print data words after the last pass
//! --census                 count every access outside SRAM
//! ```
//! Evidence: `docs/research/fx-oracle-live.md`.
use nord_sharc::core::Core;
use nord_sharc::runner::audio::wav_bytes;
use nord_sharc::runner::fx_run::{
    calls_at, fx_live_run, read_wav, regs_at, write_block, FxCore, FRAMES_PER_PASS, FX1, FX2,
};
use nord_sharc::runner::msg::{parse_msg, parse_script, Msg};
use nord_sharc::runner::parse_number;
use nord_sharc::runner::periph_run::PeriphOpts;
use nord_sharc::runner::system::FRAME_RATE;
use std::fmt::Write as _;
use std::process::ExitCode;

/// How the input signal is made.
#[derive(Clone, Debug, PartialEq)]
enum Source {
    Sine(f32, f32),
    Impulse(f32),
    Wav(String),
}

#[derive(Debug)]
struct Options {
    image: String,
    core: FxCore,
    passes: u32,
    boot: u64,
    msgs: Vec<Msg>,
    slots: Vec<u32>,
    source: Source,
    rate: u32,
    lead: u32,
    tap: Option<u32>,
    wav: Option<String>,
    dumps: Vec<(u32, u32)>,
    regs_at: Option<u32>,
    calls_at: Option<u32>,
    periph: PeriphOpts,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        image: String::new(),
        core: FX2,
        passes: 2000,
        boot: 3_000_000,
        msgs: Vec::new(),
        slots: Vec::new(),
        source: Source::Sine(440.0, 0.25),
        rate: FRAME_RATE as u32,
        lead: 0,
        tap: None,
        wav: None,
        dumps: Vec::new(),
        regs_at: None,
        calls_at: None,
        periph: PeriphOpts::default(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
        let num = |s: String| parse_number(&s).ok_or(format!("bad value {s:?} for {a}"));
        match a.as_str() {
            "--core" => {
                o.core = match val()?.as_str() {
                    "1" => FX1,
                    "2" => FX2,
                    c => return Err(format!("--core wants 1 or 2, not {c:?}")),
                }
            }
            "--passes" => o.passes = num(val()?)?,
            "--boot" => o.boot = u64::from(num(val()?)?),
            "--script-flat" => {
                let f = val()?;
                let text = std::fs::read_to_string(&f).map_err(|e| format!("{f}: {e}"))?;
                let mut v = parse_script(&text)?;
                v.iter_mut().for_each(|m| m.pass = 0);
                o.msgs.extend(v);
            }
            "--msg" => o.msgs.push(parse_msg(&val()?)?),
            "--slot" => o.slots.push(num(val()?)?),
            "--in-wav" => o.source = Source::Wav(val()?),
            "--sine" => {
                let s = val()?;
                let (f, g) = s.split_once(':').ok_or("--sine needs HZ:AMP")?;
                o.source = Source::Sine(
                    f.parse().map_err(|_| "bad --sine Hz")?,
                    g.parse().map_err(|_| "bad --sine amplitude")?,
                );
            }
            "--impulse" => {
                o.source = Source::Impulse(val()?.parse().map_err(|_| "bad --impulse")?);
            }
            "--rate" => o.rate = num(val()?)?,
            "--lead" => o.lead = num(val()?)?,
            "--tap" => o.tap = Some(num(val()?)?),
            "--wav" => o.wav = Some(val()?),
            "--regs-at" => o.regs_at = Some(num(val()?)?),
            "--calls-at" => o.calls_at = Some(num(val()?)?),
            "--dump" => {
                let s = val()?;
                let (x, n) = s.split_once(':').ok_or("--dump needs ADDR:COUNT")?;
                o.dumps.push((
                    parse_number(x).ok_or("bad --dump address")?,
                    parse_number(n).ok_or("bad --dump count")?,
                ));
            }
            _ if o.periph.take_flag(a) => {}
            _ if o.image.is_empty() && !a.starts_with("--") => o.image = a.clone(),
            _ => return Err(format!("unknown argument {a:?}")),
        }
    }
    if o.image.is_empty() {
        return Err(
            "usage: sharc-fxlive IMAGE [--passes N] [--script-flat F] (see the module docs)".into(),
        );
    }
    if o.slots.is_empty() {
        o.slots.push(o.core.input_base);
    }
    Ok(o)
}

/// Stereo input of `frames` frames for `source` at `rate`.
fn make_input(source: &Source, frames: usize, rate: u32) -> Result<(Vec<f32>, Vec<f32>), String> {
    let mut l = vec![0f32; frames];
    match source {
        Source::Sine(hz, amp) => {
            for (k, x) in l.iter_mut().enumerate() {
                *x = amp * (std::f32::consts::TAU * hz * k as f32 / rate as f32).sin();
            }
            Ok((l.clone(), l))
        }
        Source::Impulse(amp) => {
            if let Some(x) = l.first_mut() {
                *x = *amp;
            }
            Ok((l.clone(), l))
        }
        Source::Wav(f) => {
            let bytes = std::fs::read(f).map_err(|e| format!("{f}: {e}"))?;
            let (ch, _) = read_wav(&bytes)?;
            let pick = |c: usize| {
                let mut v = ch.get(c).or(ch.first()).cloned().unwrap_or_default();
                v.resize(frames, 0.0);
                v
            };
            Ok((pick(0), pick(1)))
        }
    }
}

fn stats(v: &[f32]) -> (f32, f64) {
    let peak = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    let rms = (v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / v.len().max(1) as f64).sqrt();
    (peak, rms)
}

fn run(args: &[String]) -> Result<String, String> {
    let o = parse_args(args)?;
    let data = std::fs::read(&o.image).map_err(|e| format!("{}: {e}", o.image))?;
    let mut core = o.core.boot(&data, o.boot, &o.periph)?;
    let frames = o.passes as usize * FRAMES_PER_PASS;
    let lead = o.lead as usize * FRAMES_PER_PASS;
    let (mut il, mut ir) = make_input(&o.source, frames.saturating_sub(lead), o.rate)?;
    il.splice(0..0, vec![0.0; lead]);
    ir.splice(0..0, vec![0.0; lead]);
    let slots = o.slots.clone();
    let mut inject = |p: u32, c: &mut Core| {
        let k = p as usize * FRAMES_PER_PASS;
        for &s in &slots {
            write_block(
                c,
                s,
                &il[k..k + FRAMES_PER_PASS],
                &ir[k..k + FRAMES_PER_PASS],
            );
        }
    };
    let r = fx_live_run(&o.core, &mut core, &o.msgs, o.passes, o.tap, &mut inject)?;
    let (pk_in, rms_in) = stats(&il);
    let (pk, rms) = stats(&r.left);
    let (pkr, rmsr) = stats(&r.right);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} passes, {} frames, steps first {} last {}",
        r.passes,
        r.left.len(),
        r.first_steps,
        r.last_steps
    );
    let _ = writeln!(out, "input L peak {pk_in:.6} rms {rms_in:.6}");
    let _ = writeln!(
        out,
        "output L peak {pk:.6} rms {rms:.6}; R peak {pkr:.6} rms {rmsr:.6}"
    );
    if core.sdram_reads + core.sdram_writes > 0 {
        let _ = writeln!(
            out,
            "sdram window 0x08000000: {} reads, {} writes",
            core.sdram_reads, core.sdram_writes
        );
    }
    if let Some(pc) = o.calls_at {
        let _ = core.dm_write(o.core.ping_select, 0);
        let n = calls_at(&mut core, &o.core.config(), pc);
        let _ = writeln!(out, "calls of 0x{pc:x} in one pass: {n}");
    }
    if let Some(pc) = o.regs_at {
        let _ = core.dm_write(o.core.ping_select, 0);
        match regs_at(&mut core, &o.core.config(), pc) {
            Some((m, i)) => {
                let _ = writeln!(
                    out,
                    "regs at 0x{pc:x}: M {m:x?}
                 I {i:x?}"
                );
                let at_i0: Vec<f32> = (0..8)
                    .map(|k| f32::from_bits(core.dm_read(i[0].wrapping_add(k)).unwrap_or(0)))
                    .collect();
                let _ = writeln!(out, "                 DM(I0..+8) {at_i0:?}");
            }
            None => {
                let _ = writeln!(out, "pc 0x{pc:x} not reached in a pass");
            }
        }
    }
    for &(addr, n) in &o.dumps {
        let w: Vec<String> = (0..n)
            .map(|k| format!("{:x}", core.dm_read(addr.wrapping_add(k)).unwrap_or(0)))
            .collect();
        let _ = writeln!(out, "DM(0x{addr:x}..): {}", w.join(" "));
    }
    if let Some(f) = &o.wav {
        std::fs::write(f, wav_bytes(&[r.left, r.right], o.rate))
            .map_err(|e| format!("{f}: {e}"))?;
        let _ = writeln!(out, "wrote {f}");
        if o.tap.is_some() {
            let t = format!("{f}.tap.wav");
            std::fs::write(&t, wav_bytes(&[r.tap_left, r.tap_right], o.rate))
                .map_err(|e| format!("{t}: {e}"))?;
            let _ = writeln!(out, "wrote {t}");
        }
    }
    if let Some(l) = core.access_log() {
        let _ = write!(out, "census:\n{}", l.table());
    }
    Ok(out)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn calls_at_parses() {
        let o = parse_args(&args(&["img", "--calls-at", "0x124AF2"])).unwrap();
        assert_eq!(o.calls_at, Some(0x12_4AF2));
        assert!(parse_args(&args(&["img", "--calls-at"])).is_err());
    }

    #[test]
    fn options_parse_sources_and_reject_bad_input() {
        let o = parse_args(&args(&[
            "img", "--sine", "100:0.5", "--slot", "0xC0188", "--msg", "0x10=f1", "--lead", "7",
        ]))
        .unwrap();
        assert_eq!(
            (o.source, o.slots, o.msgs.len()),
            (Source::Sine(100.0, 0.5), vec![0xC_0188], 1)
        );
        let d = parse_args(&args(&["img", "--impulse", "0.5"])).unwrap();
        assert_eq!(
            (d.slots, d.source),
            (vec![FX2.input_base], Source::Impulse(0.5))
        );
        let c1 = parse_args(&args(&["img", "--core", "1"])).unwrap();
        assert_eq!(c1.slots, vec![FX1.input_base]);
        assert!(parse_args(&args(&["img", "--core", "3"])).is_err());
        for bad in [
            &["img", "--passes"][..],
            &["--passes", "1"],
            &["img", "--bogus"],
            &["img", "--sine", "5"],
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn input_sources_make_stereo_blocks() {
        let (l, r) = make_input(&Source::Impulse(0.5), 8, 48_000).unwrap();
        assert_eq!((l[0], l[1], l.len(), l == r), (0.5, 0.0, 8, true));
        let (s, _) = make_input(&Source::Sine(12_000.0, 1.0), 8, 48_000).unwrap();
        assert!((s[1] - 1.0).abs() < 1e-5 && s[0] == 0.0);
        assert!(make_input(&Source::Wav("no_such.wav".into()), 4, 1).is_err());
        let (pk, rms) = stats(&[0.5, -1.0]);
        assert_eq!(pk, 1.0);
        assert!((rms - (0.625f64).sqrt()).abs() < 1e-9);
    }

    #[test]
    fn bad_runs_fail() {
        assert!(run(&args(&[])).is_err());
        assert!(run(&args(&["no_such_file.bin"])).is_err());
    }
}
