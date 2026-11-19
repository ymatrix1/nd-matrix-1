//! `cargo run --release -p nord-tools --bin fx_live_native -- FAMILY IN.wav OUT.wav [options]`
//!
//! Runs one FX family of the native engine (`nord_core::fx`) on a stereo WAV, the counterpart of
//! `sharc-fxlive` (the SHARC oracle running the firmware FX image on the same input). The
//! comparison is done by `tools/fx_oracle_live.py`; method and results:
//! `docs/research/fx-oracle-live.md`. Engine defaults are untouched (D22).
//!
//! ```text
//! FAMILY   reverb0   Spring reverb (kRvbType 0, FwReverb, 40-bit registers) at amount words
//!                    send 1 / dry 0 (--send S --dry D to change)
//!          chor4     Mod 2 CHOR (kFx2Type 4) through LayerFxChain with FirmwareFx::mod2
//!                    (--rate R --amount A, raw 0..127, default 64 64)
//!          mod1 / mod2  any firmware Mod 1 / Mod 2 port (`--ty N` = kFx1Type / kFx2Type,
//!                    `--rate R --amount A` raw 0..127; mod1 `--pass-frames 4` runs the RM / A-WAH /
//!                    WAH / PUMP routines on 4 frames per call like the live pass), the families of `tools/fx_vs_oracle.py`
//!          comp      firmware compressor (`kCmpAmt` `--amount` 0..127), `0x120630`
//!          rvbm      firmware multi-tap reverb (`kRvbType` `--ty` 2..11 = Booth, Room, Stage, Hall,
//!                    Cathedral and their variations) at `--amount` 0..127 through `LayerFxChain`
//!          off       the same chain with Mod 2 off: must be the identity (self check)
//! ```
//! The chain runs at the firmware frame rate 43,402.34375 Hz so that no rate scaling applies.
use nord_core::fx::{
    FirmwareFx, FwReverb, LayerFxChain, Mod1Type, Mod2Type, ReverbColor, ReverbModel,
};
use nord_tools::wav_io::{read_wav, write_wav};

/// DSP frame rate in Hz (`docs/research/dsp-frame-rate.md`).
const FRAME_RATE: f32 = 43_402.344;

/// Reads `--name value` as `T`, `default` when absent.
fn opt<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> Result<T, String> {
    match args.iter().position(|a| a == name) {
        None => Ok(default),
        Some(i) => args
            .get(i + 1)
            .and_then(|v| v.parse().ok())
            .ok_or(format!("bad value for {name}")),
    }
}

/// Renders `family` over the stereo input.
fn render(
    family: &str,
    input: &[Vec<f32>],
    args: &[String],
) -> Result<(Vec<f32>, Vec<f32>), String> {
    let l = input.first().cloned().unwrap_or_default();
    let r = input.get(1).cloned().unwrap_or_else(|| l.clone());
    let n = l.len().min(r.len());
    let (mut ol, mut or) = (vec![0f32; n], vec![0f32; n]);
    match family {
        "reverb0" => {
            let (send, dry): (f32, f32) = (opt(args, "--send", 1.0)?, opt(args, "--dry", 0.0)?);
            let mut rv = FwReverb::new(0);
            rv.set_amount_words(send, dry, true);
            for b in 0..n / 4 {
                let mut f: [[f32; 2]; 4] = std::array::from_fn(|k| [l[4 * b + k], r[4 * b + k]]);
                rv.process_block4(&mut f);
                for (k, fr) in f.iter().enumerate() {
                    (ol[4 * b + k], or[4 * b + k]) = (fr[0], fr[1]);
                }
            }
        }
        "rvbm" => {
            let ty = opt(args, "--ty", 8u8)?;
            let mut fx = LayerFxChain::new(FRAME_RATE);
            fx.firmware_fx = FirmwareFx::OFF;
            fx.firmware_fx.reverb = true;
            fx.reverb_model = match ty {
                2 | 3 => ReverbModel::Booth,
                4 | 5 => ReverbModel::Room,
                6 | 7 => ReverbModel::Stage,
                8 | 9 => ReverbModel::Hall,
                10 | 11 => ReverbModel::Cathedral,
                _ => return Err(format!("no multi-tap reverb type {ty}")),
            };
            fx.reverb_variation = ty & 1 == 1;
            fx.reverb_color = ReverbColor::Normal;
            fx.reverb_mix = opt(args, "--amount", 127.0f32)? / 127.0;
            for k in 0..n {
                [ol[k], or[k]] = fx.process_frame([l[k], r[k]]);
            }
        }
        "comp" => {
            let mut fx = LayerFxChain::new(FRAME_RATE);
            fx.firmware_fx = FirmwareFx::OFF;
            fx.bypass_reverb = true;
            fx.firmware_fx.compressor = true;
            fx.comp_enabled = true;
            fx.comp_amount = opt(args, "--amount", 64.0f32)? / 127.0;
            for k in 0..n {
                [ol[k], or[k]] = fx.process_frame([l[k], r[k]]);
            }
        }
        "mod1" => {
            let mut fx = LayerFxChain::new(FRAME_RATE);
            fx.firmware_fx = FirmwareFx::OFF;
            fx.bypass_reverb = true;
            fx.firmware_fx.mod1 = true;
            fx.mod1_type = Mod1Type::Tremolo;
            fx.mod1_type_raw = Some(opt(args, "--ty", 2u8)?);
            fx.mod1_rate_raw = Some(opt(args, "--rate", 64u8)?);
            fx.mod1_rest_frames = opt(args, "--pass-frames", 2usize)?;
            fx.mod1_amount = opt(args, "--amount", 64.0f32)? / 127.0;
            for k in 0..n {
                [ol[k], or[k]] = fx.process_frame([l[k], r[k]]);
            }
        }
        "mod2" => {
            let mut fx = LayerFxChain::new(FRAME_RATE);
            fx.firmware_fx = FirmwareFx::OFF;
            fx.bypass_reverb = true;
            fx.firmware_fx.mod2 = true;
            fx.mod2_type = Mod2Type::Chorus;
            fx.mod2_type_raw = Some(opt(args, "--ty", 4u8)?);
            fx.mod2_rate_raw = Some(opt(args, "--rate", 64u8)?);
            fx.mod2_amount = opt(args, "--amount", 64.0f32)? / 127.0;
            for k in 0..n {
                [ol[k], or[k]] = fx.process_frame([l[k], r[k]]);
            }
        }
        "chor4" | "chor5" | "off" => {
            let mut fx = LayerFxChain::new(FRAME_RATE);
            fx.firmware_fx = FirmwareFx::OFF;
            fx.bypass_reverb = true;
            if family != "off" {
                fx.firmware_fx.mod2 = true;
                fx.mod2_type = Mod2Type::Chorus;
                fx.mod2_type_raw = Some(if family == "chor5" { 5 } else { 4 });
                fx.mod2_rate_raw = Some(opt(args, "--rate", 64u8)?);
                fx.mod2_amount = opt(args, "--amount", 64.0f32)? / 127.0;
            }
            for k in 0..n {
                [ol[k], or[k]] = fx.process_frame([l[k], r[k]]);
            }
        }
        _ => return Err(format!("unknown family {family:?} (reverb0, chor4, off)")),
    }
    Ok((ol, or))
}

fn run(args: &[String]) -> Result<(), String> {
    let (family, inp, out) = match args {
        [f, i, o, ..] => (f.as_str(), i, o),
        _ => return Err("usage: fx_live_native FAMILY IN.wav OUT.wav [options]".into()),
    };
    let bytes = std::fs::read(inp).map_err(|e| format!("{inp}: {e}"))?;
    let (ch, rate) = read_wav(&bytes)?;
    let (l, r) = render(family, &ch, &args[3..])?;
    std::fs::write(out, write_wav(&[l, r], rate)).map_err(|e| format!("{out}: {e}"))
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize) -> Vec<Vec<f32>> {
        let l: Vec<f32> = (0..n).map(|i| 0.25 * (i as f32 * 0.07).sin()).collect();
        vec![l.clone(), l]
    }

    #[test]
    fn the_off_chain_is_the_identity_and_unknown_families_fail() {
        let x = sine(64);
        let (l, r) = render("off", &x, &[]).unwrap();
        assert_eq!((l, r), (x[0].clone(), x[1].clone()));
        assert!(render("nope", &x, &[]).is_err());
    }

    #[test]
    fn reverb_and_chorus_are_live_and_delayed_by_four_frames() {
        let x = sine(12_000);
        for fam in ["reverb0", "chor4"] {
            let (l, _) = render(fam, &x, &[]).unwrap();
            assert!(l[..4].iter().all(|v| *v == 0.0), "{fam}: 4 frames latency");
            assert!(
                l[6_000..].iter().any(|v| v.abs() > 1e-3),
                "{fam} is not silent"
            );
            assert!(l.iter().all(|v| v.is_finite()), "{fam}");
        }
    }

    #[test]
    fn generic_mod_families_are_live_for_every_wired_type() {
        let x = sine(12_000);
        for (fam, ty) in [
            ("mod1", "2"),
            ("mod1", "5"),
            ("mod1", "10"),
            ("mod2", "0"),
            ("mod2", "8"),
        ] {
            let a: Vec<String> = ["--ty", ty].iter().map(|s| s.to_string()).collect();
            let (l, _) = render(fam, &x, &a).unwrap();
            assert!(l.iter().all(|v| v.is_finite()), "{fam} {ty}");
            assert!(
                l[6_000..].iter().any(|v| v.abs() > 1e-3),
                "{fam} {ty} silent"
            );
        }
    }

    #[test]
    fn options_parse_and_reject_garbage() {
        let a: Vec<String> = ["--send", "0.5"].iter().map(|s| s.to_string()).collect();
        assert_eq!(opt(&a, "--send", 1.0f32), Ok(0.5));
        assert_eq!(opt(&a, "--dry", 0.25f32), Ok(0.25));
        let bad: Vec<String> = ["--send", "x"].iter().map(|s| s.to_string()).collect();
        assert!(opt(&bad, "--send", 1.0f32).is_err());
        assert!(run(&[]).is_err());
    }
}
