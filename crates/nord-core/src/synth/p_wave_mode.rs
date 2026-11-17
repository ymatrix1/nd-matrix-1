//! Table row of a decoded oscillator, by oscillator mode (D140).
//!
//! ANALOG waves pick their row from `(category, wave)` ([`PWave::from_category_wave`]); the FM
//! mode and the SAMPLE / WAVETABLE modes have a row of their own so the level of the ANALOG
//! `Other` row can move (`--synth-p oracle`) without changing the sample path, which the oracle
//! stream does not reach (`docs/research/synth-osc-gain-oracle.md` section 5).

use super::{PWave, SynthOscMode};

impl PWave {
    /// Row of an oscillator: ANALOG by `(category, wave)`, FM and sampled modes by mode.
    pub fn for_osc_mode(mode: SynthOscMode, category: u8, wave: u8) -> Self {
        match mode {
            SynthOscMode::Analog => Self::from_category_wave(category, wave),
            SynthOscMode::Fm => Self::Fm,
            SynthOscMode::Sample | SynthOscMode::Wavetable => Self::Sampled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{PTableSpec, SynthChain, SynthLayer, VaWaveform};

    /// Steady RMS of a sine note, filter off, in the strip chain with the given table row.
    fn sine_rms(table: PTableSpec, wave: PWave) -> f32 {
        let mut s = SynthLayer::new(48_000.0);
        s.chain = SynthChain::FirmwareStrip;
        s.filter_on = false;
        s.waveform = VaWaveform::Sine;
        s.p_table = Some(table.table());
        s.p_wave = wave;
        s.note_on(60, 127);
        let mut acc = 0.0f64;
        for i in 0..24_000 {
            let [l, _] = s.render_frame();
            if i >= 12_000 {
                acc += f64::from(l * l);
            }
        }
        (acc / 12_000.0).sqrt() as f32
    }

    #[test]
    fn oracle_table_lowers_the_idle_rows_but_not_the_fitted_or_sample_rows() {
        let base = PTableSpec::BUILTIN;
        let ratio = |w| sine_rms(PTableSpec::oracle(), w) / sine_rms(base, w);
        for w in [PWave::ShapeSquare, PWave::Other, PWave::Fm] {
            assert!((ratio(w) - 0.4).abs() < 0.01, "{w:?} {}", ratio(w));
        }
        for w in [PWave::Sampled, PWave::PureSaw, PWave::Noise] {
            assert!((ratio(w) - 1.0).abs() < 1e-6, "{w:?}");
        }
    }

    #[test]
    fn modes_have_their_own_rows_and_analog_keeps_the_wave_rows() {
        assert_eq!(
            PWave::for_osc_mode(SynthOscMode::Analog, 0, 2),
            PWave::PureSaw
        );
        assert_eq!(
            PWave::for_osc_mode(SynthOscMode::Analog, 0, 0),
            PWave::Other
        );
        assert_eq!(PWave::for_osc_mode(SynthOscMode::Fm, 3, 1), PWave::Fm);
        assert_eq!(
            PWave::for_osc_mode(SynthOscMode::Sample, 0, 0),
            PWave::Sampled
        );
        assert_eq!(
            PWave::for_osc_mode(SynthOscMode::Wavetable, 2, 2),
            PWave::Sampled
        );
    }
}
