//! Level and spectrum metrics of one rendered note for the program ranking (`program_rank`,
//! `docs/research/program-ranking.md`). Offline tooling: allocates freely.
//!
//! Definitions (tool conventions, ASSUMED; they are the same for every program so only
//! differences between two renders carry meaning):
//! - level: `20 log10(rms)` of the mono mean `(L + R) / 2` over the analysed frames;
//! - centroid: power-weighted mean frequency of the Hann periodogram between 20 Hz and 20 kHz;
//! - bands: share of the periodogram power in 8 bands, in dB relative to the total power.

use crate::band_response::power_spectrum;

/// Band edges in Hz (8 bands; the last one ends at the 20 kHz analysis limit).
pub const BAND_EDGES_HZ: [f64; 9] = [
    20.0, 100.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 20_000.0,
];

/// Floor of a band share in dB (an empty band reads this, not minus infinity).
/// ASSUMED: tool convention, far below any band that carries a note.
pub const BAND_FLOOR_DB: f64 = -90.0;

/// Number of frames analysed (a power of two: 2.73 s at 48 kHz).
/// ASSUMED: tool convention, same for every program (see `HOLD` in `program_rank`).
pub const ANALYSIS_FRAMES: usize = 131_072;

/// Metrics of one mono signal.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteMetrics {
    /// RMS level in dB (floored at -200).
    pub rms_db: f64,
    /// Peak level in dB (floored at -200).
    pub peak_db: f64,
    /// Spectral centroid in Hz, `None` for a silent signal.
    pub centroid_hz: Option<f64>,
    /// Power share of each band in dB relative to the total power.
    pub bands_db: [f64; 8],
}

fn to_db(x: f64) -> f64 {
    if x > 1e-10 {
        20.0 * x.log10()
    } else {
        -200.0
    }
}

/// Mono mean of a stereo pair.
pub fn mono(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).map(|(l, r)| 0.5 * (l + r)).collect()
}

/// Metrics of the first `ANALYSIS_FRAMES` frames of `x` (all of it when shorter, rounded down to a
/// power of two by the periodogram).
pub fn note_metrics(x: &[f32], rate: f64) -> NoteMetrics {
    let x = &x[..x.len().min(ANALYSIS_FRAMES)];
    let n = x.len().max(1);
    let rms = (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / n as f64).sqrt();
    let peak = x.iter().fold(0.0f64, |m, v| m.max(f64::from(v.abs())));
    if x.len() < 2 {
        return NoteMetrics {
            rms_db: to_db(rms),
            peak_db: to_db(peak),
            centroid_hz: None,
            bands_db: [BAND_FLOOR_DB; 8],
        };
    }
    let p = power_spectrum(x);
    let bin_hz = rate / (2 * p.len()) as f64;
    let mut bands = [0.0f64; 8];
    let (mut total, mut weighted) = (0.0f64, 0.0f64);
    for (k, pw) in p.iter().enumerate() {
        let f = k as f64 * bin_hz;
        if !(BAND_EDGES_HZ[0]..BAND_EDGES_HZ[8]).contains(&f) {
            continue;
        }
        total += pw;
        weighted += pw * f;
        if let Some(b) = (0..8).find(|&b| f < BAND_EDGES_HZ[b + 1]) {
            bands[b] += pw;
        }
    }
    let silent = total <= 0.0 || rms < 1e-9;
    NoteMetrics {
        rms_db: to_db(rms),
        peak_db: to_db(peak),
        centroid_hz: (!silent).then(|| weighted / total),
        bands_db: bands.map(|b| {
            if silent || b <= 0.0 {
                BAND_FLOOR_DB
            } else {
                (10.0 * (b / total).log10()).max(BAND_FLOOR_DB)
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (std::f64::consts::TAU * freq * i as f64 / 48_000.0).sin() as f32)
            .collect()
    }

    #[test]
    fn sine_level_centroid_and_band() {
        let m = note_metrics(&sine(1_500.0, 65_536, 0.5), 48_000.0);
        assert!((m.rms_db - 20.0 * (0.5f64 / 2f64.sqrt()).log10()).abs() < 0.05);
        assert!((m.peak_db - 20.0 * 0.5f64.log10()).abs() < 0.05);
        let c = m.centroid_hz.unwrap();
        assert!((c - 1_500.0).abs() < 20.0, "centroid {c}");
        assert!(
            m.bands_db[4] > -0.1,
            "1.5 kHz sits in band 4: {:?}",
            m.bands_db
        );
        assert!(m.bands_db[0] <= BAND_FLOOR_DB + 1.0);
    }

    #[test]
    fn silence_has_no_centroid_and_floors() {
        let m = note_metrics(&vec![0.0; 4096], 48_000.0);
        assert_eq!(m.rms_db, -200.0);
        assert!(m.centroid_hz.is_none());
        assert!(m.bands_db.iter().all(|b| *b == BAND_FLOOR_DB));
    }

    #[test]
    fn band_shares_sum_to_one() {
        let x: Vec<f32> = (0..32_768)
            .map(|i| 0.3 * (0.07 * i as f32).sin() + 0.2 * (1.9 * i as f32).sin())
            .collect();
        let m = note_metrics(&x, 48_000.0);
        let s: f64 = m.bands_db.iter().map(|b| 10f64.powf(b / 10.0)).sum();
        assert!((s - 1.0).abs() < 1e-3, "sum {s}");
    }

    #[test]
    fn mono_averages_channels_and_short_input_is_safe() {
        assert_eq!(mono(&[1.0, 0.0], &[0.0, 1.0]), vec![0.5, 0.5]);
        let m = note_metrics(&[0.25], 48_000.0);
        assert!(m.centroid_hz.is_none());
        let e = note_metrics(&[], 48_000.0);
        assert_eq!(e.rms_db, -200.0);
    }

    #[test]
    fn louder_signal_reads_higher() {
        let a = note_metrics(&sine(440.0, 16_384, 0.1), 48_000.0);
        let b = note_metrics(&sine(440.0, 16_384, 0.2), 48_000.0);
        assert!((b.rms_db - a.rms_db - 6.02).abs() < 0.05);
    }
}
