//! `cargo run --release -p nord-tools --bin fx_reverb_types [-- --mix 1.0]`
//!
//! Firmware multi-tap reverb (`FirmwareFx::reverb`, `kRvbType` 2..11) against the engine FDN, one
//! row per type: first echo, wet energy over input, T60 per octave band (impulse), band gains
//! (white noise). Findings: `docs/research/reverb-fw-vs-hw.md`. Nothing here reads firmware or
//! audio files.
use nord_core::fx::{FirmwareFx, LayerFxChain, ReverbColor, ReverbModel};
use nord_tools::band_response::{noise, power_spectrum};
use nord_tools::reverb_measure::{energy, first_echo, noise_gains, rt60_bands, t60, RT_BANDS};

const FS: f64 = 48_000.0;
const IR: usize = 1 << 19;
const NOISE: usize = 1 << 17;
const GAIN_BANDS: [(&str, f64, f64); 5] = [
    ("dc-60", 0.0, 60.0),
    ("60-250", 60.0, 250.0),
    ("0.5-2k", 500.0, 2000.0),
    ("4-8k", 4000.0, 8000.0),
    ("8-16k", 8000.0, 16_000.0),
];

fn chain(fw: bool, model: ReverbModel, var: bool, mix: f32) -> LayerFxChain {
    let mut fx = LayerFxChain::new(FS as f32);
    fx.firmware_fx = FirmwareFx {
        reverb: fw,
        ..FirmwareFx::OFF
    };
    fx.reverb_model = model;
    fx.reverb_variation = var;
    fx.reverb_color = ReverbColor::Normal;
    fx.reverb_mix = mix;
    fx
}

fn run(fw: bool, model: ReverbModel, var: bool, mix: f32, name: &str) {
    let mut fx = chain(fw, model, var, mix);
    let ir: Vec<f32> = (0..IR)
        .map(|k| fx.process_frame([f32::from(k == 0), f32::from(k == 0)])[0])
        .collect();
    let wet_e = energy(&ir[64..]);
    let rt = rt60_bands(&ir[64..], FS);
    let mut fx = chain(fw, model, var, mix);
    let x = noise(NOISE, 7, 0.1);
    let y: Vec<f32> = x.iter().map(|&v| fx.process_frame([v, v])[0]).collect();
    let bands: Vec<(f64, f64)> = GAIN_BANDS.iter().map(|b| (b.1, b.2)).collect();
    let g = noise_gains(&x[8192..], &y[8192..], FS, &bands);
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let rts: Vec<String> = rt.iter().map(|t| format!("{t:5.2}")).collect();
    let gs: Vec<String> = g.iter().map(|t| format!("{t:6.1}")).collect();
    println!(
        "{name:<14} {} echo {:5.1} ms E {:6.1} dB | T60 {:5.2} | bands {} | gain {} | peak {peak:.2}",
        if fw { "fw " } else { "fdn" },
        first_echo(&ir[1..], 0.05) as f64 / FS * 1000.0,
        10.0 * wet_e.log10(),
        t60(&ir[64..], FS),
        rts.join(" "),
        gs.join(" ")
    );
}

/// Band power of a power spectrum of a signal sampled at `FS` (bins `n/2`).
fn band_power(p: &[f64], lo: f64, hi: f64) -> f64 {
    let bin = FS / (2.0 * p.len() as f64);
    p.iter()
        .enumerate()
        .filter(|(k, _)| (*k as f64 * bin) >= lo && (*k as f64 * bin) < hi)
        .map(|(_, v)| v)
        .sum()
}

/// Floor of the firmware routine: a 220 Hz sine through the chain; power outside the sine in
/// sub (20-60 Hz), brilliance (3-8 kHz) and air (8-16 kHz), in dB re the sine's power. The
/// routine is not a linear time-invariant system (packed 16-bit delay words), so a pure tone
/// leaves a noise floor that a very clean dry signal does not have.
fn floor(model: ReverbModel, var: bool, mix: f32, name: &str) {
    let mut fx = chain(true, model, var, mix);
    let n = 1 << 18;
    let x: Vec<f32> = (0..n)
        .map(|i| 0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / FS as f32).sin())
        .collect();
    let y: Vec<f32> = x.iter().map(|&v| fx.process_frame([v, v])[0]).collect();
    let (px, py) = (power_spectrum(&x[16384..]), power_spectrum(&y[16384..]));
    let re = band_power(&px, 100.0, 400.0);
    let db = |lo, hi| 10.0 * (band_power(&py, lo, hi) / re).log10();
    println!(
        "{name:<14} floor re sine at mix {mix}: sub {:6.1} dB, brilliance {:6.1} dB, air {:6.1} dB",
        db(20.0, 60.0),
        db(3000.0, 8000.0),
        db(8000.0, 16_000.0)
    );
}

fn main() {
    let mut mix = 1.0f32;
    let a: Vec<String> = std::env::args().collect();
    if let Some(i) = a.iter().position(|s| s == "--mix") {
        mix = a[i + 1].parse().expect("mix");
    }
    println!(
        "mix {mix}; T60 bands {:?}; gain bands {:?}",
        RT_BANDS.map(|b| b.0),
        GAIN_BANDS.map(|b| b.0)
    );
    let types = [
        (ReverbModel::Booth, "Booth"),
        (ReverbModel::Room, "Room"),
        (ReverbModel::Stage, "Stage"),
        (ReverbModel::Hall, "Hall"),
        (ReverbModel::Cathedral, "Cathedral"),
    ];
    for (m, n) in types {
        for var in [false, true] {
            let name = format!("{n}{}", if var { " var" } else { "" });
            run(true, m, var, mix, &name);
            run(false, m, var, mix, &name);
            floor(m, var, mix, &name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_power_of_a_sine_sits_in_its_band() {
        let x: Vec<f32> = (0..16_384)
            .map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / FS as f32).sin())
            .collect();
        let p = power_spectrum(&x);
        assert!(band_power(&p, 900.0, 1100.0) > 100.0 * band_power(&p, 2000.0, 4000.0));
    }

    #[test]
    fn the_firmware_chain_differs_from_the_engine_chain() {
        let mut a = chain(true, ReverbModel::Cathedral, false, 1.0);
        let mut b = chain(false, ReverbModel::Cathedral, false, 1.0);
        let ya: Vec<f32> = (0..2000)
            .map(|k| a.process_frame([f32::from(k == 0); 2])[0])
            .collect();
        let yb: Vec<f32> = (0..2000)
            .map(|k| b.process_frame([f32::from(k == 0); 2])[0])
            .collect();
        assert!(ya != yb);
    }
}
