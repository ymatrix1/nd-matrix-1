//! `fx_engine_vs_fw [--out FILE]`: every FX family of the layer chain through the engine path and
//! the firmware port on four deterministic stimuli (impulse, sweep, noise, held notes); one CSV
//! row per case and stimulus with level, octave band and centroid differences. The ports are
//! bit-checked against the SHARC oracle (`docs/research/fx-vs-oracle.md`), so the columns read
//! "engine minus firmware". Numbers only; no audio or firmware content is written.
use nord_tools::fx_cases::{bands_db, cases, centroid, chain, render, rms_db, stimulus, FS};
use nord_tools::reverb_measure::{first_echo, t60};
use std::fmt::Write as _;

const STIMULI: [&str; 4] = ["impulse", "sweep", "noise", "notes"];
const HEADER: &str = "family,case,stim,rms_in_db,rms_engine_db,rms_fw_db,delta_db,peak_engine,peak_fw,centroid_engine_hz,centroid_fw_hz,echo_engine_ms,echo_fw_ms,t60_engine_s,t60_fw_s,band_delta_db";

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

fn main() -> std::process::ExitCode {
    let a: Vec<String> = std::env::args().collect();
    let out = a
        .iter()
        .position(|s| s == "--out")
        .and_then(|i| a.get(i + 1));
    let n = 1usize << 16;
    let mut text = format!("{HEADER}\n");
    for case in cases().iter().filter(|c| !c.port.is_empty()) {
        for stim in STIMULI {
            let (x, _) = stimulus(stim, n);
            let ye = render(&mut chain(case, false), &x);
            let yf = render(&mut chain(case, true), &x);
            let (ee, ef) = (first_echo(&ye[1..], 0.05), first_echo(&yf[1..], 0.05));
            let bd: Vec<String> = bands_db(&x, &ye)
                .iter()
                .zip(bands_db(&x, &yf))
                .map(|(e, f)| format!("{:.1}", e - f))
                .collect();
            let ms = |k: usize| k as f64 / f64::from(FS) * 1000.0;
            let _ = writeln!(
                text,
                "{},{},{stim},{:.2},{:.2},{:.2},{:.2},{:.3},{:.3},{:.0},{:.0},{:.1},{:.1},{:.2},{:.2},{}",
                case.family,
                case.label,
                rms_db(&x),
                rms_db(&ye),
                rms_db(&yf),
                rms_db(&ye) - rms_db(&yf),
                peak(&ye),
                peak(&yf),
                centroid(&ye),
                centroid(&yf),
                ms(ee),
                ms(ef),
                if stim == "impulse" { t60(&ye[64..], f64::from(FS)) } else { 0.0 },
                if stim == "impulse" { t60(&yf[64..], f64::from(FS)) } else { 0.0 },
                bd.join(" ")
            );
        }
    }
    match out {
        Some(p) => match std::fs::write(p, text) {
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
    fn header_has_as_many_columns_as_a_row_writes() {
        assert_eq!(HEADER.split(',').count(), 16);
        assert_eq!(peak(&[0.1, -0.7, 0.3]), 0.7);
    }
}
