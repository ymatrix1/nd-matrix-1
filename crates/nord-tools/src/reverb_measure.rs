//! Measurements of a reverb stage: per-band RT60 from an impulse response, band gains from noise,
//! pre-delay and wet-only level (used by `fx_reverb_types`, `docs/research/reverb-fw-vs-hw.md`).
//! Nothing here reads firmware or audio files.

use crate::band_response::band_gain_db;

/// Octave bands (lo, hi) in Hz used for the RT60 table.
pub const RT_BANDS: [(f64, f64); 6] = [
    (60.0, 250.0),
    (250.0, 500.0),
    (500.0, 1000.0),
    (1000.0, 2000.0),
    (2000.0, 4000.0),
    (4000.0, 8000.0),
];

/// Band-pass of `x` for `[lo, hi)` Hz: two cascaded RBJ band-pass biquads (0 dB peak) centred on
/// the geometric mean with `Q = fc / (hi - lo)`. A smooth filter (an FFT brick wall rings into the
/// exactly-zero tail of a finite response and fakes a decay floor).
pub fn band_limit(x: &[f32], rate: f64, lo: f64, hi: f64) -> Vec<f32> {
    let fc = (lo.max(1.0) * hi).sqrt();
    let w0 = std::f64::consts::TAU * fc / rate;
    let alpha = w0.sin() / (2.0 * fc / (hi - lo));
    let a0 = 1.0 + alpha;
    let (b0, b2) = (alpha / a0, -alpha / a0);
    let (a1, a2) = (-2.0 * w0.cos() / a0, (1.0 - alpha) / a0);
    let mut y: Vec<f64> = x.iter().map(|v| f64::from(*v)).collect();
    for _ in 0..2 {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        for v in &mut y {
            let o = b0 * *v + b2 * x2 - a1 * y1 - a2 * y2;
            (x2, x1, y2, y1) = (x1, *v, y1, o);
            *v = o;
        }
    }
    y.iter().map(|v| *v as f32).collect()
}

/// T60 in seconds from the Schroeder backward integral of `x`, fitted between `-5` and `-25` dB
/// (T20 extrapolated by 3). `NaN` when the decay never reaches -25 dB.
pub fn t60(x: &[f32], rate: f64) -> f64 {
    let mut edc = vec![0.0f64; x.len()];
    let mut acc = 0.0;
    for (i, v) in x.iter().enumerate().rev() {
        acc += f64::from(*v).powi(2);
        edc[i] = acc;
    }
    let total = edc.first().copied().unwrap_or(0.0);
    if total <= 0.0 {
        return f64::NAN;
    }
    let at = |level: f64| {
        edc.iter()
            .position(|&v| 10.0 * (v / total).log10() <= level)
    };
    match (at(-5.0), at(-25.0)) {
        (Some(a), Some(b)) => 3.0 * (b - a) as f64 / rate,
        _ => f64::NAN,
    }
}

/// Index of the first sample with `|x| >= frac * peak`.
pub fn first_echo(x: &[f32], frac: f32) -> usize {
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    x.iter().position(|v| v.abs() >= frac * peak).unwrap_or(0)
}

/// Total energy of `x` (sum of squares).
pub fn energy(x: &[f32]) -> f64 {
    x.iter().map(|v| f64::from(*v).powi(2)).sum()
}

/// RT60 per [`RT_BANDS`] of an impulse response.
pub fn rt60_bands(ir: &[f32], rate: f64) -> Vec<f64> {
    RT_BANDS
        .iter()
        .map(|&(lo, hi)| t60(&band_limit(ir, rate, lo, hi), rate))
        .collect()
}

/// Gain in dB (output over input power) per `(lo, hi)` band of a noise run.
pub fn noise_gains(input: &[f32], output: &[f32], rate: f64, bands: &[(f64, f64)]) -> Vec<f64> {
    bands
        .iter()
        .map(|&(lo, hi)| band_gain_db(input, output, rate, lo, hi))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exp_decay(n: usize, rate: f64, t60_s: f64) -> Vec<f32> {
        let mut s = 1u32;
        (0..n)
            .map(|i| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let w = (s >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
                w * 10f64.powf(-3.0 * i as f64 / rate / t60_s) as f32
            })
            .collect()
    }

    #[test]
    fn t60_recovers_a_known_decay() {
        let x = exp_decay(96_000, 48_000.0, 0.8);
        let t = t60(&x, 48_000.0);
        assert!((t - 0.8).abs() < 0.08, "{t}");
    }

    #[test]
    fn band_limit_keeps_a_tone_inside_and_drops_one_outside() {
        let n = 8192;
        let tone = |f: f64| -> Vec<f32> {
            (0..n)
                .map(|i| (std::f64::consts::TAU * f * i as f64 / 48_000.0).sin() as f32)
                .collect()
        };
        let inside = band_limit(&tone(1500.0), 48_000.0, 1000.0, 2000.0);
        let outside = band_limit(&tone(3000.0), 48_000.0, 1000.0, 2000.0);
        let e = |v: &[f32]| energy(&v[2048..]);
        assert!(e(&inside) > 0.8 * e(&tone(1500.0)));
        assert!(e(&outside) < 0.2 * e(&tone(3000.0)));
    }

    #[test]
    fn first_echo_and_energy() {
        let mut x = vec![0.0f32; 100];
        x[40] = 0.5;
        x[60] = 1.0;
        assert_eq!(first_echo(&x, 0.4), 40);
        assert_eq!(first_echo(&x, 0.9), 60);
        assert!((energy(&x) - 1.25).abs() < 1e-9);
    }

    #[test]
    fn noise_gain_of_a_scaled_copy_is_the_scale_in_db() {
        let x = crate::band_response::noise(16_384, 3, 0.1);
        let y: Vec<f32> = x.iter().map(|v| v * 0.5).collect();
        let g = noise_gains(&x, &y, 48_000.0, &[(100.0, 5000.0)]);
        assert!((g[0] + 6.02).abs() < 0.05, "{g:?}");
    }
}
