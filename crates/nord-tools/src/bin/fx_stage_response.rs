//! `cargo run --release -p nord-tools --bin fx_stage_response [-- --detail]`
//!
//! Magnitude response above 1 kHz of every downstream FX stage (amp drive and cabinet, Amp EQ,
//! layer compressor, master bus), measured with deterministic white noise (`band_response`).
//! Each stage is run three ways:
//!
//! * `fwF`   the firmware port at the DSP frame rate 43,402.34 Hz: this is the oracle response in
//!   Hz by inference (the ports are bit exact per frame against the `sharc-fx` oracle goldens and
//!   the firmware runs one frame per 1/43,402 s, `docs/research/dsp-frame-rate.md`; the oracle
//!   itself was not driven with noise through the amp),
//! * `fw48`  the same port at the engine rate 48 kHz (what a 48 kHz render does),
//! * `rm48`  the same port at 48 kHz behind `LayerFxChain::set_fx_rate_match(true)` (opt-in),
//! * `eng48` the engine's own path (RBJ biquads, `tanh`) at 48 kHz.
//!
//! Gains are output power over input power per band in dB; `d48` = `fw48 - fwF` is the error of
//! running the per-frame firmware words at 48 kHz, `dRM` = `rm48 - fwF` the same with the rate
//! match, `dEng` = `eng48 - fwF`. Findings:
//! `docs/research/brightness-downstream-oracle.md`. Nothing here reads firmware or audio.
use nord_core::fx::{AmpCabinet, FirmwareFx, LayerFxChain, MasterBus};
use nord_core::organ::FIRMWARE_FRAME_RATE_HZ;
use nord_tools::band_response::{band_gain_db, noise, third_octave_bands};

const FRAMES: usize = 1 << 17;
const SKIP: usize = 4096;
/// Aggregate bands of `tools/video_ref.py` above 1 kHz (mid upper edge, presence, brilliance, air).
const AGG: [(&str, f64, f64); 4] = [
    ("1-1.5k", 1000.0, 1500.0),
    ("pres 1.5-3k", 1500.0, 3000.0),
    ("brill 3-8k", 3000.0, 8000.0),
    ("8-16k", 8000.0, 16_000.0),
];

type Setup = Box<dyn Fn(&mut LayerFxChain, bool)>;

/// Gains in dB of `frame_fn` driven with noise at `rms` (stereo, decorrelated) for each band.
fn gains(
    rate: f64,
    rms: f32,
    bands: &[(f64, f64)],
    mut f: impl FnMut([f32; 2]) -> [f32; 2],
) -> Vec<f64> {
    let (l, r) = (noise(FRAMES + SKIP, 11, rms), noise(FRAMES + SKIP, 12, rms));
    let out: Vec<[f32; 2]> = (0..FRAMES + SKIP).map(|i| f([l[i], r[i]])).collect();
    let lo: Vec<f32> = out[SKIP..].iter().map(|o| o[0]).collect();
    bands
        .iter()
        .map(|&(a, b)| band_gain_db(&l[SKIP..], &lo, rate, a, b))
        .collect()
}

fn chain(rate: f64, setup: &Setup, firmware: bool) -> LayerFxChain {
    let mut fx = LayerFxChain::new(rate as f32);
    fx.firmware_fx = FirmwareFx::OFF;
    fx.bypass_reverb = true;
    setup(&mut fx, firmware);
    fx
}

fn row(rms: f32, setup: &Setup, bands: &[(f64, f64)]) -> [Vec<f64>; 4] {
    let f_hz = FIRMWARE_FRAME_RATE_HZ;
    let run = |rate: f64, firmware: bool, rm: bool| {
        let mut fx = chain(rate, setup, firmware);
        fx.set_fx_rate_match(rm);
        gains(rate, rms, bands, |x| fx.process_frame(x))
    };
    [
        run(f_hz, true, false),
        run(48_000.0, true, false),
        run(48_000.0, true, true),
        run(48_000.0, false, false),
    ]
}

fn print_row(name: &str, g: &[Vec<f64>; 4], bands: &[&str]) {
    for (i, b) in bands.iter().enumerate() {
        let (f, n, r, e) = (g[0][i], g[1][i], g[2][i], g[3][i]);
        let flag = if (n - f).abs() >= 3.0 {
            " <== d48 >= 3 dB"
        } else {
            ""
        };
        println!(
            "{name:34} {b:>11} fwF {f:7.2} fw48 {n:7.2} rm48 {r:7.2} eng48 {e:7.2} | d48 {:6.2} dRM {:6.2} dEng {:6.2}{flag}",
            n - f,
            r - f,
            e - f
        );
    }
}

fn amp_setups() -> Vec<(String, Setup)> {
    let mut v: Vec<(String, Setup)> = Vec::new();
    for (cab, name) in [
        (AmpCabinet::Twin, "Twin"),
        (AmpCabinet::Jc, "JC"),
        (AmpCabinet::Small, "Small"),
    ] {
        for var in [false, true] {
            for drive in [0.0f32, 0.11, 0.37] {
                let label = format!("amp {name}{} drive {drive}", if var { " II" } else { " I" });
                v.push((
                    label,
                    Box::new(move |fx, fw| {
                        fx.amp_cabinet = cab;
                        fx.amp_variation = var;
                        fx.drive_amount = drive;
                        fx.firmware_fx.amp = fw;
                    }),
                ));
            }
        }
    }
    v
}

fn eq_setups() -> Vec<(String, Setup)> {
    // (label, bass dB, mid dB, mid Hz, treble dB): Tiny Transistor, A:25 mid cut, flat, shelf up.
    let sets = [
        ("eq Tiny Transistor", 8.75, -1.5, 1350.0, -5.0),
        ("eq A:25 mid -6 dB", 0.0, -6.0, 1500.0, 0.0),
        ("eq treble +6 dB", 0.0, 0.0, 1000.0, 6.0),
        ("eq treble -6 dB", 0.0, 0.0, 1000.0, -6.0),
        ("eq mid +6 dB @3 kHz", 0.0, 6.0, 3000.0, 0.0),
    ];
    sets.iter()
        .map(|&(n, b, m, mf, t)| {
            let s: Setup = Box::new(move |fx, fw| {
                fx.eq_bass_db = b;
                fx.eq_mid_db = m;
                fx.eq_mid_freq_hz = mf;
                fx.eq_treble_db = t;
                fx.firmware_fx.eq = fw;
            });
            (n.to_string(), s)
        })
        .collect()
}

fn comp_setups() -> Vec<(String, Setup)> {
    [0.25f32, 0.5, 1.0]
        .iter()
        .map(|&a| {
            let s: Setup = Box::new(move |fx, fw| {
                fx.comp_enabled = true;
                fx.comp_amount = a;
                fx.firmware_fx.compressor = fw;
            });
            (format!("layer compressor amount {a}"), s)
        })
        .collect()
}

fn master_rows(bands: &[(f64, f64)], names: &[&str]) {
    let f_hz = FIRMWARE_FRAME_RATE_HZ;
    let run = |rate: f64, iir: bool, fw_comp: bool| {
        let mut m = MasterBus::default();
        m.master_iir_enabled = iir;
        if fw_comp {
            m.use_fw_compressor_amount();
        }
        gains(rate, 0.05, bands, |x| m.process_frame(x))
    };
    let cases = [
        (
            "master default (recon LP only)",
            run(48_000.0, false, false),
        ),
        ("master + firmware IIR @48k", run(48_000.0, true, false)),
        ("master + firmware IIR @F", run(f_hz, true, false)),
        (
            "master fw compressor amount .25",
            run(48_000.0, false, true),
        ),
    ];
    for (n, g) in cases {
        for (i, b) in names.iter().enumerate() {
            println!("{n:34} {b:>11} gain {:7.2} dB", g[i]);
        }
    }
}

fn main() {
    let detail = std::env::args().any(|a| a == "--detail");
    let (bands, names): (Vec<(f64, f64)>, Vec<String>) = if detail {
        let b = third_octave_bands();
        let n = b
            .iter()
            .map(|(l, h)| format!("{:.0}", (l * h).sqrt()))
            .collect();
        (b, n)
    } else {
        (
            AGG.iter().map(|a| (a.1, a.2)).collect(),
            AGG.iter().map(|a| a.0.to_string()).collect(),
        )
    };
    let nm: Vec<&str> = names.iter().map(String::as_str).collect();
    println!("# amp (drive + cabinet), noise -30 dBFS rms, firmware port vs engine path");
    for (label, s) in amp_setups() {
        print_row(&label, &row(0.0316, &s, &bands), &nm);
    }
    println!("# Amp EQ (firmware tone stack vs engine biquads)");
    for (label, s) in eq_setups() {
        print_row(&label, &row(0.0316, &s, &bands), &nm);
    }
    println!("# layer compressor (firmware gain computer vs engine), noise -18 dBFS rms");
    for (label, s) in comp_setups() {
        print_row(&label, &row(0.126, &s, &bands), &nm);
    }
    println!("# master bus");
    master_rows(&bands, &nm);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_chain_has_zero_gain_in_every_aggregate_band() {
        let setup: Setup = Box::new(|_, _| {});
        let bands: Vec<(f64, f64)> = AGG.iter().map(|a| (a.1, a.2)).collect();
        let g = row(0.05, &setup, &bands);
        for run in &g {
            for v in run {
                assert!(v.abs() < 0.05, "{v}");
            }
        }
    }

    #[test]
    fn running_the_twin_cabinet_at_48k_is_brighter_than_at_the_frame_rate() {
        let setups = amp_setups();
        let (_, s) = setups
            .iter()
            .find(|(n, _)| n == "amp Twin I drive 0")
            .expect("twin setup");
        let bands = [(3000.0, 8000.0)];
        let g = row(0.0316, s, &bands);
        assert!(g[1][0].is_finite() && g[0][0].is_finite());
        assert!(g[1][0] != g[0][0], "the two rates must differ");
    }

    #[test]
    fn setup_tables_have_the_expected_sizes() {
        assert_eq!(amp_setups().len(), 18);
        assert_eq!(eq_setups().len(), 5);
        assert_eq!(comp_setups().len(), 3);
    }
}
