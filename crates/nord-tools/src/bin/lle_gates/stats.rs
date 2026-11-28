//! Sample statistics of `lle-gates`: per-run figures, min-max bands over runs, and the noise rule.
//!
//! The rule is the repository's A/B protocol (`arm_sb_ab.rs`, `docs/research/arm0-jit-design.md`
//! section 12): engines are compared by the median of their per-run medians, the band of an engine
//! is the min-max of that figure over its runs, and two engines are "within noise" when their
//! medians differ by less than the larger of the two bands.

/// Percentile `p` in `[0, 1]` of `v` (nearest rank on the sorted copy); 0 for an empty slice.
pub fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((s.len() - 1) as f64 * p).round() as usize]
}

/// Mean, median, p99 and max of one run's samples (all in the unit of the samples).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunStats {
    pub mean: f64,
    pub median: f64,
    pub p99: f64,
    pub max: f64,
}

impl RunStats {
    pub fn of(v: &[f64]) -> Self {
        let mean = if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        };
        Self {
            mean,
            median: percentile(v, 0.5),
            p99: percentile(v, 0.99),
            max: percentile(v, 1.0),
        }
    }
}

/// Median and min-max of one figure over the runs of an engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub min: f64,
    pub median: f64,
    pub max: f64,
}

impl Band {
    pub fn of(v: &[f64]) -> Self {
        Self {
            min: percentile(v, 0.0),
            median: percentile(v, 0.5),
            max: percentile(v, 1.0),
        }
    }

    pub fn width(&self) -> f64 {
        self.max - self.min
    }

    /// `median [min..max]` with `dp` decimals.
    pub fn text(&self, dp: usize) -> String {
        format!(
            "{:.dp$} [{:.dp$}..{:.dp$}]",
            self.median, self.min, self.max
        )
    }
}

/// How two engines compare on one figure (lower is better).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Compare {
    /// The medians differ by less than the larger band: no difference can be claimed.
    WithinNoise,
    /// `a` is lower by this factor (`b / a`, above 1).
    AFaster(f64),
    /// `b` is lower by this factor (`a / b`, above 1).
    BFaster(f64),
}

pub fn compare(a: &Band, b: &Band) -> Compare {
    let noise = a.width().max(b.width());
    if (a.median - b.median).abs() < noise || a.median <= 0.0 || b.median <= 0.0 {
        return Compare::WithinNoise;
    }
    if a.median < b.median {
        Compare::AFaster(b.median / a.median)
    } else {
        Compare::BFaster(a.median / b.median)
    }
}

impl Compare {
    /// One phrase for the table.
    pub fn text(&self, a: &str, b: &str) -> String {
        match self {
            Compare::WithinNoise => format!("{a} vs {b}: within noise"),
            Compare::AFaster(x) => format!("{a} vs {b}: {a} faster x{x:.2}"),
            Compare::BFaster(x) => format!("{a} vs {b}: {b} faster x{x:.2}"),
        }
    }
}

/// Fraction of `v` strictly below `limit` (1.0 for an empty slice: nothing was late).
pub fn share_below(v: &[f64], limit: f64) -> f64 {
    if v.is_empty() {
        return 1.0;
    }
    v.iter().filter(|x| **x < limit).count() as f64 / v.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_the_nearest_rank_and_survive_empty_input() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(
            (
                percentile(&v, 0.0),
                percentile(&v, 0.5),
                percentile(&v, 1.0)
            ),
            (1.0, 51.0, 100.0)
        );
        assert_eq!(percentile(&v, 0.99), 99.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn run_stats_are_mean_median_p99_max() {
        let r = RunStats::of(&[1.0, 2.0, 3.0, 10.0]);
        assert_eq!((r.mean, r.max), (4.0, 10.0));
        assert_eq!(RunStats::of(&[]).mean, 0.0);
    }

    #[test]
    fn a_band_is_the_median_and_the_min_max_of_the_runs() {
        let b = Band::of(&[5.0, 3.0, 4.0]);
        assert_eq!((b.min, b.median, b.max, b.width()), (3.0, 4.0, 5.0, 2.0));
        assert_eq!(b.text(1), "4.0 [3.0..5.0]");
    }

    #[test]
    fn medians_closer_than_the_larger_band_are_within_noise() {
        let a = Band::of(&[100.0, 104.0, 108.0]); // median 104, band 8
        let b = Band::of(&[101.0, 110.0, 112.0]); // median 110, band 11
        assert_eq!(compare(&a, &b), Compare::WithinNoise, "gap 6 < 11");
        assert_eq!(compare(&b, &a), Compare::WithinNoise);
        assert!(compare(&a, &b).text("a", "b").contains("within noise"));
    }

    #[test]
    fn a_gap_wider_than_both_bands_names_the_faster_engine_and_the_ratio() {
        let a = Band::of(&[50.0, 51.0, 52.0]);
        let b = Band::of(&[100.0, 101.0, 103.0]);
        match compare(&a, &b) {
            Compare::AFaster(x) => assert!((x - 101.0 / 51.0).abs() < 1e-9),
            c => panic!("{c:?}"),
        }
        assert!(matches!(compare(&b, &a), Compare::BFaster(_)));
        assert!(compare(&a, &b)
            .text("uop", "aot")
            .contains("uop faster x1.98"));
    }

    #[test]
    fn the_noise_limit_is_strict_and_a_zero_median_claims_nothing() {
        let a = Band::of(&[10.0, 12.0]); // median 12 (nearest rank, rounds up), band 2
        let b = Band::of(&[14.0, 14.0]); // median 14, band 0: gap 2 is not below 2
        assert!(matches!(compare(&a, &b), Compare::AFaster(_)));
        let z = Band::of(&[0.0, 0.0]);
        assert_eq!(compare(&z, &b), Compare::WithinNoise);
    }

    #[test]
    fn share_below_counts_strictly_below_the_limit() {
        assert_eq!(share_below(&[0.5, 1.0, 2.0, 0.9], 1.0), 0.5);
        assert_eq!(share_below(&[], 1.0), 1.0);
    }
}
