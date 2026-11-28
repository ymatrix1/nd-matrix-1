//! Firmware WAVE bank (kOscBank 3, Core2 stage `0x1247B6`) as an opt-in model.
//!
//! Spec: `docs/specs/synth-engine.md` section 4.3; evidence `docs/research/synth-stage-models-13.md`;
//! bit-exact Python model `tools/synth_stage_wave.py` (320 of 320 oracle words).
//!
//! The stage is the two-oscillator sibling of SUPER 2..4 ([`super::fw_super2`]): the same band
//! row, 6-word buffer, 512-phase 8-tap FIR, `a a b b` hold and three-biquad output, with two
//! oscillators instead of four voices. It reads the table at `DM(0xC15A5)`; the ARM fills that
//! slot with the selected wave of the 46-record registry in the OS image (table id = wave id).
//! Only the boot slot (wave id 0, `Second`) is available as derived numbers, so every WAVE layer
//! plays that table until the per-wave loader transform is decoded (OQ22).

use super::fw_stages::fade;
use std::sync::OnceLock;

use super::fw_super2::{band_index, prepare, tables, voice_value, Super24Tables};

/// ASSUMED (the P block ratios are open, OQ22): both oscillators at the pitch.
pub const WAVE_RATIOS: [f32; 2] = [1.0, 1.0];
/// ASSUMED (P block weights open): equal weights summing to one.
pub const WAVE_WEIGHTS: [f32; 2] = [0.5, 0.5];
/// VERIFIED (listing `0x248F6C..0x248F7A`): the pitch word is clipped to this before use.
const PITCH_CLIP: f32 = 0.5;

/// State of one WAVE bank voice oscillator (no heap).
#[derive(Debug, Clone, Copy, Default)]
pub struct WaveBankState {
    /// Oscillator phases (2^32 = one cycle).
    pub phase: [u32; 2],
    /// The mixed value held for two output samples.
    pub held: f32,
    /// Second sample of the hold.
    pub second: bool,
    /// Biquad states `s1, s2` of the three sections.
    pub bq: [f32; 6],
}

/// Parses the tables before the first rendered sample (allocation outside the audio thread).
pub fn prepare_wave_bank() {
    prepare();
}

static WAVES: OnceLock<Vec<Super24Tables>> = OnceLock::new();

/// Installs the 46 tables read from the user's OS image (`fw_wave_tables`); once per process.
pub fn install_wave_tables(t: Vec<Super24Tables>) -> bool {
    WAVES.set(t).is_ok()
}

/// Whether the user's tables are installed (`install_wave_tables` ran): the plugin enables the
/// WAVE stage only then, so a host without the OS image keeps the placeholder path.
pub fn wave_tables_installed() -> bool {
    WAVES.get().is_some()
}

/// The table of wave id `id`: the installed one, else the boot table (wave id 0, `Second`).
pub fn wave_table(id: usize) -> &'static Super24Tables {
    WAVES.get().and_then(|t| t.get(id)).unwrap_or_else(tables)
}

/// One output sample for the render increment `dt` (cycles per sample) and SY increment `i_sy`.
pub fn wave_bank_sample(st: &mut WaveBankState, t: &Super24Tables, dt: f32, i_sy: f32) -> f32 {
    if !st.second {
        let f = (2.0 * i_sy).min(PITCH_CLIP);
        let idx = band_index(f).min(t.rows.len() / 4 - 1);
        let (offset, count) = (t.rows[4 * idx] as usize, t.rows[4 * idx + 1] as usize);
        let mut sum = 0.0;
        for ((ph, rho), w) in st.phase.iter_mut().zip(WAVE_RATIOS).zip(WAVE_WEIGHTS) {
            // two output samples per held value: the phase advances by twice the sample step
            let step = 2.0 * dt * rho;
            *ph = ph.wrapping_add((step.fract() as f64 * 4_294_967_296.0) as u64 as u32);
            sum += w * voice_value(t, *ph, offset, count);
        }
        st.held = sum * fade(2.0 * i_sy);
    }
    st.second = !st.second;
    let c = &t.lattice;
    let mut x = st.held;
    for n in 0..3 {
        let (s1, s2) = (st.bq[2 * n], st.bq[2 * n + 1]);
        let y = c[0] * x + s1;
        st.bq[2 * n] = c[1] * x + s2 - c[3] * y;
        st.bq[2 * n + 1] = c[2] * x - c[4] * y;
        x = y;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dt: f32, i_sy: f32, n: usize) -> Vec<f32> {
        prepare_wave_bank();
        let mut st = WaveBankState::default();
        (0..n)
            .map(|_| wave_bank_sample(&mut st, tables(), dt, i_sy))
            .collect()
    }

    #[test]
    fn the_installed_flag_answers_without_installing() {
        // The process-wide `OnceLock` may already hold tables from another test: the flag only
        // reports, and `wave_table` always answers.
        let _ = wave_tables_installed();
        assert!(wave_table(0).rows.len() >= 4);
    }

    #[test]
    fn the_stage_is_finite_bounded_deterministic_and_pitched() {
        let (dt, i_sy) = (220.0 / 48_000.0, 220.0 / 43_402.34);
        let a = run(dt, i_sy, 4000);
        let b = run(dt, i_sy, 4000);
        assert!(a.iter().all(|x| x.is_finite() && x.abs() < 8.0));
        assert_eq!(
            a.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            b.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        );
        let rms = (a.iter().map(|x| x * x).sum::<f32>() / a.len() as f32).sqrt();
        assert!(rms > 1e-3, "silent: {rms}");
        let crossings = a[1000..]
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count() as f32;
        let cycles = 3000.0 * 220.0 / 48_000.0;
        assert!(
            crossings >= 0.8 * cycles && crossings <= 2.3 * cycles,
            "{crossings}"
        );
    }

    #[test]
    fn the_output_holds_each_value_for_two_samples_before_the_biquads() {
        // with the biquads at rest the hold shows as equal increments of the input: the first two
        // outputs are `b0 x` and `b0 x + s1` of the same held value
        prepare_wave_bank();
        let mut st = WaveBankState::default();
        let _ = wave_bank_sample(&mut st, tables(), 0.004, 0.005);
        let held = st.held;
        assert!(st.second);
        let _ = wave_bank_sample(&mut st, tables(), 0.004, 0.005);
        assert_eq!(st.held.to_bits(), held.to_bits());
        assert!(!st.second);
    }

    #[test]
    fn the_phase_advances_by_two_samples_per_held_value() {
        prepare_wave_bank();
        let mut st = WaveBankState::default();
        let dt = 0.01;
        let _ = wave_bank_sample(&mut st, tables(), dt, 0.005);
        let want = (2.0 * dt * 4_294_967_296.0) as u32;
        assert!(st.phase[0].abs_diff(want) <= 4_096, "{}", st.phase[0]);
        assert_eq!(st.phase[0], st.phase[1]);
    }

    #[test]
    fn the_flag_switches_wavetable_layers_to_the_stage() {
        use super::super::{SynthChain, SynthLayer, SynthOscMode};
        let render = |flag: bool| -> Vec<u32> {
            let mut s = SynthLayer::new(48_000.0);
            s.chain = SynthChain::FirmwareStrip;
            s.filter_on = false;
            s.osc_mode = SynthOscMode::Wavetable;
            s.wave_bank = flag;
            prepare_wave_bank();
            s.note_on(60, 127);
            (0..6000).map(|_| s.render_frame()[0].to_bits()).collect()
        };
        let (off, on) = (render(false), render(true));
        assert_ne!(off, on, "the flag must change the oscillator");
        assert_eq!(on, render(true), "deterministic");
        assert!(on.iter().any(|b| f32::from_bits(*b).abs() > 1e-4), "silent");
    }

    #[test]
    fn a_higher_pitch_changes_the_band_and_stays_bounded() {
        let lo = run(100.0 / 48_000.0, 100.0 / 43_402.34, 2000);
        let hi = run(3000.0 / 48_000.0, 3000.0 / 43_402.34, 2000);
        assert!(hi.iter().all(|x| x.is_finite() && x.abs() < 8.0));
        assert_ne!(lo[1500].to_bits(), hi[1500].to_bits());
    }
}
