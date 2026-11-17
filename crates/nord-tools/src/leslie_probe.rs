//! Measurement helpers of the `leslie_probe` binary: amplitude modulation depth and rate of a
//! stereo output lane, and the ARM rotor speed words.
//!
//! The firmware Leslie (`0x1256AD`) is driven directly with test signals; nothing here reads
//! firmware, samples or audio. Findings: `docs/research/leslie-gate.md`.

use std::f64::consts::PI;

/// DSP frame rate of the firmware, 44,444,000 / 1024 Hz (`docs/research/dsp-frame-rate.md`).
pub const FS_DSP: f64 = 44_444_000.0 / 1024.0;
/// Frames per envelope block (about 2.95 ms at [`FS_DSP`]).
pub const BLOCK: usize = 128;

/// ARM speed word of a nominal rotor rate: `int(hz * 8388607 / 11025) << 8`
/// (VERIFIED, `docs/research/arm-part-activation.md`; slow 0.80 Hz gives `0x26000`).
pub fn speed_word(nominal_hz: f64) -> u32 {
    ((nominal_hz * 8_388_607.0 / 11_025.0).trunc().max(0.0) as u32) << 8
}

/// The three words `[0xB2F25 rotor, 0xB2F26 horn, 0xB2FA1 horn follower]` for nominal rates;
/// the follower is `0.5748 x` the horn (EMPIRICAL, `organ/rotary.rs`).
pub fn speed_words(rotor_hz: f64, horn_hz: f64) -> [u32; 3] {
    [
        speed_word(rotor_hz),
        speed_word(horn_hz),
        speed_word(horn_hz * 0.5748),
    ]
}

/// RMS of consecutive blocks of [`BLOCK`] samples.
pub fn block_rms(x: &[f32]) -> Vec<f64> {
    x.as_chunks::<BLOCK>()
        .0
        .iter()
        .map(|c| (c.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / BLOCK as f64).sqrt())
        .collect()
}

/// Peak-to-peak depth `(p95 - p5) / (p95 + p5)` of an envelope (0 = steady, 1 = full).
pub fn am_depth(env: &[f64]) -> f64 {
    if env.is_empty() {
        return 0.0;
    }
    let mut s = env.to_vec();
    s.sort_by(f64::total_cmp);
    let at = |q: f64| s[((s.len() - 1) as f64 * q).round() as usize];
    let (hi, lo) = (at(0.95), at(0.05));
    if hi + lo <= 0.0 {
        0.0
    } else {
        (hi - lo) / (hi + lo)
    }
}

/// Strongest envelope component in `lo..hi` Hz: `(rate Hz, depth)` with depth = 2 |X| / (N mean).
/// `block_rate` is the envelope sample rate (`FS_DSP / BLOCK`). Direct DFT, 0.05 Hz grid.
pub fn envelope_peak(env: &[f64], block_rate: f64, lo: f64, hi: f64) -> (f64, f64) {
    let n = env.len() as f64;
    let mean = env.iter().sum::<f64>() / n.max(1.0);
    if mean <= 0.0 {
        return (0.0, 0.0);
    }
    let mut best = (0.0, 0.0);
    let mut f = lo;
    while f <= hi {
        let (mut re, mut im) = (0.0, 0.0);
        for (k, e) in env.iter().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * PI * k as f64 / n).cos();
            let a = 2.0 * PI * f * k as f64 / block_rate;
            re += (e - mean) * w * a.cos();
            im += (e - mean) * w * a.sin();
        }
        let depth = 2.0 * (re * re + im * im).sqrt() / (n * 0.5) / mean;
        if depth > best.1 {
            best = (f, depth);
        }
        f += 0.05;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_horn_word_is_the_arm_word() {
        assert_eq!(speed_word(0.80), 0x26000);
        assert_eq!(speed_word(0.73), 0x22b00);
        assert_eq!(speed_words(0.73, 0.80), [0x22b00, 0x26000, 0x15d00]);
    }

    #[test]
    fn envelope_peak_finds_a_known_modulation() {
        let br = FS_DSP / BLOCK as f64;
        let env: Vec<f64> = (0..(br * 4.0) as usize)
            .map(|k| 1.0 + 0.3 * (2.0 * PI * 6.0 * k as f64 / br).sin())
            .collect();
        let (hz, depth) = envelope_peak(&env, br, 0.3, 20.0);
        assert!((hz - 6.0).abs() < 0.15, "rate {hz}");
        assert!((depth - 0.3).abs() < 0.03, "depth {depth}");
        assert!(am_depth(&env) > 0.25 && am_depth(&[1.0; 50]) < 1e-9);
    }

    #[test]
    fn block_rms_of_a_constant_is_the_constant() {
        let r = block_rms(&[0.5f32; 3 * BLOCK]);
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|v| (v - 0.5).abs() < 1e-9));
    }
}
