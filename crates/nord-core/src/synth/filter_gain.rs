//! Native filter level by filter type (opt-in `--synth-filter-gain`, D195).
//!
//! The ORG core 2 filter stage (real ARM stream, 18 factory programs) reads louder than the native
//! filters. The gap is the ratio form `(oracle filtered / unfiltered RMS) - (native filtered /
//! unfiltered RMS)`, which cancels the oscillator, the envelope and the voice gain, so it isolates
//! the filter. This module is that gap by [`FilterModel`], applied as a linear factor on the
//! filter output. It replaces the single coupling factor `COUPLED_FILTER_GAIN` of D194.
//!
//! The per type means come from `docs/research/data/synth-filter-gain-oracle.csv` and are
//! recomputed by `cargo run --release -p nord-tools --bin synth_filter_gain`; a test in
//! `nord-tools` checks this table against the CSV.

use crate::synth_types::FilterModel;

/// Gain in dB of the oracle filter over the native one, per filter type.
///
/// | Type | dB | Programs | Spread (dB) | Tag |
/// |---|---|---|---|---|
/// | `Lp12` | `+4.65` | 5 (the resonance 0.95 outlier `Glow Chords Whl` `+15.2` excluded) | `+2.8 .. +5.8` | VERIFIED mean |
/// | `Lp24` | `+5.26` | 7 | `+2.7 .. +6.7` | VERIFIED mean |
/// | `Ladder24` | `+1.17` | 4 | `-2.2 .. +5.3` | EMPIRICAL (bimodal: cutoffs 370 / 1500 Hz `+3.1 / +5.3`, 2500 / 13000 Hz `-1.6 / -2.2`) |
/// | `BandPass` | `0.00` | 1 (`Filter Bank Ld`, `+0.02`) | n/a | VERIFIED, one program |
/// | `HighPass` | `+3.62` | 4 (`HP Splash Whl` `+7.3`, `Replica Lead` `+4.0`, `Chip Bass` `+3.7`, `Distorted Whl II` `-0.6` at the 14 Hz cutoff floor, where the filter is transparent) | `-0.6 .. +7.3` | EMPIRICAL (wide spread, 4 programs) |
///
/// VERIFIED: means of `oracle_filter_gain_db - native_filter_gain_db` over the oracle runs,
/// checked by `nord-tools` test `table_matches_the_oracle_csv` and the tool `synth_filter_gain`,
/// max difference against the CSV mean 0.01 dB. The per program scatter around the type mean is
/// `+- 1.5 dB` (LP12, LP24), `+- 2.8 dB` (HighPass) and `+- 3.6 dB` (Ladder24): the level dependence of the oracle filter
/// stage is not modelled.
pub const fn filter_gain_db(model: FilterModel) -> f32 {
    match model {
        FilterModel::Lp12 => 4.65,
        FilterModel::Lp24 => 5.26,
        FilterModel::Ladder24 => 1.17,
        FilterModel::HighPass => 3.62,
        FilterModel::BandPass => 0.0,
    }
}

/// [`filter_gain_db`] as a linear factor on the filter output.
pub fn filter_gain(model: FilterModel) -> f32 {
    10f32.powf(filter_gain_db(model) / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowpass_types_gain_five_db_highpass_three_and_the_band_pass_nothing() {
        assert!((filter_gain(FilterModel::Lp24) - 1.833).abs() < 2e-3);
        assert!((filter_gain(FilterModel::Lp12) - 1.708).abs() < 2e-3);
        assert_eq!(filter_gain(FilterModel::BandPass), 1.0);
        assert!((filter_gain(FilterModel::HighPass) - 1.517).abs() < 2e-3);
    }

    #[test]
    fn ladder_sits_between_the_two_modes_of_the_oracle() {
        let g = filter_gain_db(FilterModel::Ladder24);
        assert!(g > -2.2 && g < 5.3, "{g}");
    }
}
