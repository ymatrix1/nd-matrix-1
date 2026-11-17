//! `cargo run --release -p nord-tools --bin leslie_probe`
//!
//! Drives the firmware Leslie processor `0x1256AD` (the oracle exact `FwLeslieE40` port) with test
//! signals at the slow and fast ARM rotor speeds and prints, per lane, the gain, the amplitude
//! modulation depth and the strongest modulation rate. The signal goes in the left lane (the
//! organ bus layout, `docs/research/fx-leslie.md`). No firmware, sample or audio is read.
//! Findings: `docs/research/leslie-gate.md`.
use nord_core::fx::{FwLeslieE40, RingAdvance};
use nord_tools::leslie_probe::{am_depth, block_rms, envelope_peak, speed_words, BLOCK, FS_DSP};
use std::f64::consts::PI;

const SECONDS: f64 = 8.0;
const SKIP_S: f64 = 1.0;
const SINES: [f64; 15] = [
    110.0, 220.0, 330.0, 440.0, 550.0, 660.0, 770.0, 880.0, 1100.0, 1320.0, 2000.0, 3000.0, 4000.0,
    5000.0, 8000.0,
];

/// Nominal ARM rates (VERIFIED, `docs/research/leslie-settings.md`): rotor / horn, slow and fast.
/// The two mixed rows (rotor fast with horn slow, and the reverse) show which speed word drives
/// which band.
const SPEEDS: [(&str, f64, f64); 4] = [
    ("slow", 0.73, 0.80),
    ("fast", 6.4, 7.0),
    ("rotorfast", 6.4, 0.80),
    ("hornfast", 0.73, 7.0),
];

/// Runs a sine of `hz` through the processor, returns the two output lanes after the skip.
fn run_sine(l: &mut FwLeslieE40, hz: f64, amp: f32) -> [Vec<f32>; 2] {
    let n = (SECONDS * FS_DSP) as usize;
    let skip = (SKIP_S * FS_DSP) as usize;
    let mut out = [Vec::with_capacity(n), Vec::with_capacity(n)];
    for k in 0..n {
        let x = amp * (2.0 * PI * hz * k as f64 / FS_DSP).sin() as f32;
        let y = l.process(x, 0.0);
        if k >= skip {
            out[0].push(y[0]);
            out[1].push(y[1]);
        }
    }
    out
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

fn main() {
    let amp = 0.3f32;
    println!(
        "speed hz_in lane gain_db am_depth peak_rate_hz peak_depth depth_at_drum depth_at_horn"
    );
    for (name, rotor, horn) in SPEEDS {
        for hz in SINES {
            let mut l = FwLeslieE40::with_ring(RingAdvance::ADVANCE);
            l.set_speed_words(speed_words(rotor, horn));
            let out = run_sine(&mut l, hz, amp);
            for (lane, y) in out.iter().enumerate() {
                let gain = 20.0 * (rms(y) / (f64::from(amp) / 2f64.sqrt()) + 1e-30).log10();
                let env = block_rms(y);
                let (rate, depth) = envelope_peak(&env, FS_DSP / BLOCK as f64, 0.3, 20.0);
                // physical rates: nominal x F / 44,100 (docs/research/fx-leslie.md s11)
                let k = FS_DSP / 44_100.0;
                let at = |nominal: f64| {
                    let r = nominal * k;
                    envelope_peak(&env, FS_DSP / BLOCK as f64, r - 0.25, r + 0.25).1
                };
                println!(
                    "{name} {hz:.0} {lane} {gain:.2} {:.3} {rate:.2} {depth:.3} {:.3} {:.3}",
                    am_depth(&env),
                    at(rotor),
                    at(horn)
                );
            }
        }
    }
    // impulse response length and level at both speeds (left lane in, both lanes out)
    println!("impulse speed lane peak_abs peak_index energy");
    for (name, rotor, horn) in SPEEDS {
        let mut l = FwLeslieE40::with_ring(RingAdvance::ADVANCE);
        l.set_speed_words(speed_words(rotor, horn));
        for _ in 0..(FS_DSP as usize) {
            l.process(0.0, 0.0);
        }
        let mut y = [Vec::new(), Vec::new()];
        for k in 0..4096 {
            let o = l.process(if k == 0 { 1.0 } else { 0.0 }, 0.0);
            y[0].push(o[0]);
            y[1].push(o[1]);
        }
        for (lane, v) in y.iter().enumerate() {
            let (i, p) = v.iter().enumerate().fold((0, 0.0f32), |a, (i, x)| {
                if x.abs() > a.1 {
                    (i, x.abs())
                } else {
                    a
                }
            });
            let e: f64 = v.iter().map(|x| f64::from(*x).powi(2)).sum();
            println!("impulse {name} {lane} {p:.5} {i} {e:.3e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sine_through_the_processor_gives_finite_lanes_of_equal_length() {
        let mut l = FwLeslieE40::with_ring(RingAdvance::ADVANCE);
        l.set_speed_words(speed_words(0.73, 0.80));
        let out = run_sine(&mut l, 440.0, 0.3);
        assert_eq!(out[0].len(), out[1].len());
        assert!(out[0].iter().chain(&out[1]).all(|v| v.is_finite()));
        assert!(rms(&out[0]) > 0.0 || rms(&out[1]) > 0.0);
    }
}
