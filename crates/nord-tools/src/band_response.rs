//! Band gains of a stage measured with deterministic white noise (no allocation limits: tooling).
//!
//! Used by `fx_stage_response` (`docs/research/brightness-downstream-oracle.md`): the stage is
//! driven with the same noise, the power in `[lo, hi)` Hz of output and input is compared with an
//! FFT, so the gain is a power-weighted mean over the band (the same weighting as the six-band
//! spectrum of `tools/video_ref.py`).

/// In-place radix-2 FFT of `(re, im)` pairs; `x.len()` must be a power of two.
pub fn fft(x: &mut [(f64, f64)]) {
    let n = x.len();
    assert!(n.is_power_of_two(), "fft length must be a power of two");
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            x.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (x[start + k], x[start + k + len / 2]);
                let t = (b.0 * cr - b.1 * ci, b.0 * ci + b.1 * cr);
                x[start + k] = (a.0 + t.0, a.1 + t.1);
                x[start + k + len / 2] = (a.0 - t.0, a.1 - t.1);
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
}

/// Power spectrum of `x` (Hann window, length rounded down to a power of two), `n/2` bins.
pub fn power_spectrum(x: &[f32]) -> Vec<f64> {
    let n = if x.len().is_power_of_two() {
        x.len()
    } else {
        1 << (usize::BITS - 1 - x.len().leading_zeros())
    };
    let mut buf: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            (f64::from(x[i]) * w, 0.0)
        })
        .collect();
    fft(&mut buf);
    buf[..n / 2].iter().map(|(r, i)| r * r + i * i).collect()
}

/// Deterministic uniform noise with the given RMS (LCG, same sequence on every platform).
pub fn noise(n: usize, seed: u32, rms: f32) -> Vec<f32> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(12_345);
    let amp = rms * 3f32.sqrt();
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / (1u32 << 23) as f32 - 1.0) * amp
        })
        .collect()
}

/// Gain in dB (output power over input power) in `[lo, hi)` Hz between two equally long signals
/// sampled at `rate`. Returns `NaN` when the band holds no bin or the input has no power there.
pub fn band_gain_db(input: &[f32], output: &[f32], rate: f64, lo: f64, hi: f64) -> f64 {
    let (pi, po) = (power_spectrum(input), power_spectrum(output));
    let bin_hz = rate / (2 * pi.len()) as f64;
    let (mut si, mut so) = (0.0, 0.0);
    for (k, (a, b)) in pi.iter().zip(&po).enumerate() {
        let f = k as f64 * bin_hz;
        if f >= lo && f < hi {
            si += a;
            so += b;
        }
    }
    if si > 0.0 {
        10.0 * (so / si).log10()
    } else {
        f64::NAN
    }
}

/// One-third octave band edges `(lo, hi)` around the centres 1, 1.25, ... 16 kHz.
pub fn third_octave_bands() -> Vec<(f64, f64)> {
    let centres = [
        1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0, 10_000.0,
        12_500.0, 16_000.0,
    ];
    let k = 2f64.powf(1.0 / 6.0);
    centres.iter().map(|c| (c / k, c * k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_puts_a_sine_in_its_bin() {
        let n = 1024;
        let x: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 64.0 * i as f32 / n as f32).sin())
            .collect();
        let p = power_spectrum(&x);
        let peak = p
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(k, _)| k);
        assert_eq!(peak, Some(64));
    }

    #[test]
    fn noise_is_deterministic_and_has_the_requested_rms() {
        let a = noise(40_000, 3, 0.1);
        assert_eq!(a, noise(40_000, 3, 0.1));
        assert_ne!(a, noise(40_000, 4, 0.1));
        let rms = (a.iter().map(|v| f64::from(v * v)).sum::<f64>() / a.len() as f64).sqrt();
        assert!((rms - 0.1).abs() < 0.003, "rms {rms}");
    }

    #[test]
    fn gain_of_a_scaled_copy_is_the_scale_in_db_in_every_band() {
        let x = noise(32_768, 1, 0.1);
        let y: Vec<f32> = x.iter().map(|v| v * 0.5).collect();
        for (lo, hi) in third_octave_bands() {
            let g = band_gain_db(&x, &y, 48_000.0, lo, hi);
            assert!((g + 6.0206).abs() < 1e-3, "{lo}..{hi}: {g}");
        }
    }

    #[test]
    fn a_one_pole_low_pass_cuts_the_high_band_more_than_the_low_band() {
        let x = noise(32_768, 2, 0.1);
        let mut s = 0.0f32;
        let y: Vec<f32> = x
            .iter()
            .map(|v| {
                s += 0.2 * (v - s);
                s
            })
            .collect();
        let lo = band_gain_db(&x, &y, 48_000.0, 1000.0, 1260.0);
        let hi = band_gain_db(&x, &y, 48_000.0, 10_000.0, 12_600.0);
        assert!(lo > hi + 6.0, "{lo} {hi}");
        assert!(band_gain_db(&x, &y, 48_000.0, 30_000.0, 31_000.0).is_nan());
    }
}
