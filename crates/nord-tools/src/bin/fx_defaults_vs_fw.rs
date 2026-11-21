//! `fx_defaults_vs_fw [--out FILE]`: flip evidence for `--fx-defaults firmware` (D101, OQ34). One
//! CSV row per word variant and stimulus: the chain with the engine default and with the firmware
//! default against the firmware model driven by the firmware word (rms delta, octave band error,
//! null depth, verdict). `docs/research/fx-defaults-flip.md` holds the reading. Numbers only.
use nord_tools::fx_defaults_cmp::{run, variants, verdict, FRAMES, STIMS};
use std::fmt::Write as _;

const HEADER: &str = "word,variant,stim,engine_value,fw_value,rms_delta_engine_db,rms_delta_fw_db,band_err_engine_db,band_err_fw_db,null_engine_db,null_fw_db,closer";

fn main() -> std::process::ExitCode {
    let a: Vec<String> = std::env::args().collect();
    let out = a
        .iter()
        .position(|s| s == "--out")
        .and_then(|i| a.get(i + 1));
    let mut text = format!("{HEADER}\n");
    for v in variants() {
        for stim in STIMS {
            let (e, f) = run(&v, stim, FRAMES);
            let _ = writeln!(
                text,
                "{},{},{stim},{:.4},{:.4},{:.2},{:.2},{:.2},{:.2},{:.1},{:.1},{}",
                v.word,
                v.label,
                v.values.0,
                v.values.1,
                e.rms_delta_db,
                f.rms_delta_db,
                e.band_err_db,
                f.band_err_db,
                e.null_db,
                f.null_db,
                verdict(&e, &f)
            );
        }
    }
    match out {
        Some(p) => match std::fs::write(p, &text) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{p}: {e}");
                std::process::ExitCode::FAILURE
            }
        },
        None => {
            print!("{text}");
            std::process::ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_has_one_column_per_value_of_a_row() {
        assert_eq!(HEADER.split(',').count(), 12);
        let v = &variants()[0];
        let (e, f) = run(v, "noise", 2048);
        assert!(e.band_err_db.is_finite() && f.band_err_db.is_finite());
    }
}
