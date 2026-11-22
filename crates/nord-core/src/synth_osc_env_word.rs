//! DSP word of the Osc Env Amount (`Edt_Syn_kOscEnvAmt`, `-60..60` steps) as the firmware sends
//! it to ORG core 2 common block word 12 (`DM(0xE02E8 + 32 L + 12)`), by destination (D211).
//!
//! The ARM setter (`0xC01C61F0`, a parameter object with a law selector at `+0xF0`) keeps two laws:
//!
//! | `kOscEnvDst` (Env To Pitch) | selector | word |
//! |---|---|---|
//! | 0 (Osc Ctrl) | 2 | `x / 60` |
//! | 1 (Pitch) | 1 | `sign(x) * T(127 * abs(x) / 60) / 48`, `T` = [`LFO_PITCH_SEMITONES`], linear between entries |
//!
//! The Pitch law is the firmware's pitch amount taper (the table is the LFO Pitch amount law,
//! `0..48` semitones), so the word is the semitone depth over 48. The native engine does not use
//! the word: `SynthLayer::osc_env_amount` stays the knob value `x / 60` and `render.rs` maps it to
//! its own (`EMPIRICAL`) pitch depth; this module is the verified hardware reference for that map
//! and for the morph slot's DSP rows (`synth_morph_osc_env_tests.rs`).

use crate::synth_tables::LFO_PITCH_SEMITONES;

/// Full scale of the amount knob in steps (`kOscEnvAmt` range `-60..60`).
pub const OSC_ENV_FULL_STEPS: f32 = 60.0;
/// Semitones that a Pitch word of `1.0` stands for (`T[127]`).
pub const PITCH_WORD_SEMITONES: f32 = 48.0;

/// Float32 the firmware writes to the osc env amount word for the dispatched amount `steps`
/// (clamped to `-60..60`; fractions are kept).
///
/// VERIFIED: Osc Ctrl law `steps / 60` equals the ARM word on 21 of 21 wheel rows and 242 of 242
/// static steps; the Pitch law equals it on 6 of 6 aftertouch rows and 242 of 242 static steps
/// (Time Warp Saw and Sine-Saw Plk Whl, Env To Pitch as recalled and bit-patched), max error
/// 3e-5 steps; checked by `synth_morph_osc_env_tests.rs` over `morph-fw-osc-env.csv` and
/// `morph-fw-osc-env-dst.csv`.
pub fn osc_env_dsp_word(steps: f32, to_pitch: bool) -> f32 {
    let u = (steps / OSC_ENV_FULL_STEPS).clamp(-1.0, 1.0);
    if !to_pitch {
        return u;
    }
    let pos = u.abs() * 127.0;
    let i = pos as usize;
    let semitones = if i >= 127 {
        LFO_PITCH_SEMITONES[127]
    } else {
        let lo = LFO_PITCH_SEMITONES[i];
        lo + (LFO_PITCH_SEMITONES[i + 1] - lo) * (pos - i as f32)
    };
    semitones.copysign(u) / PITCH_WORD_SEMITONES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_ctrl_law_is_linear_and_clamped() {
        assert_eq!(osc_env_dsp_word(0.0, false), 0.0);
        assert!((osc_env_dsp_word(30.0, false) - 0.5).abs() < 1e-7);
        assert!((osc_env_dsp_word(-37.5, false) + 0.625).abs() < 1e-7);
        assert_eq!(osc_env_dsp_word(90.0, false), 1.0);
        assert_eq!(osc_env_dsp_word(-90.0, false), -1.0);
    }

    #[test]
    fn pitch_law_hits_the_table_ends_and_is_odd() {
        assert_eq!(osc_env_dsp_word(0.0, true), 0.0);
        assert!((osc_env_dsp_word(60.0, true) - 1.0).abs() < 1e-6);
        assert!((osc_env_dsp_word(-60.0, true) + 1.0).abs() < 1e-6);
        for x in [1.0, 7.5, 24.0, 48.0, 59.9] {
            assert_eq!(osc_env_dsp_word(x, true), -osc_env_dsp_word(-x, true));
        }
    }

    #[test]
    fn pitch_law_matches_two_captured_words() {
        // ARM words of Time Warp Saw layer A: `-48 -> 0xBF133336`, `-24 -> 0xBDD33336`.
        assert!((osc_env_dsp_word(-48.0, true) - f32::from_bits(0xBF13_3336)).abs() < 1e-6);
        assert!((osc_env_dsp_word(-24.0, true) - f32::from_bits(0xBDD3_3336)).abs() < 1e-6);
    }

    #[test]
    fn pitch_law_is_monotone_and_below_the_linear_law() {
        let mut prev = 0.0;
        for k in 1..=60 {
            let w = osc_env_dsp_word(k as f32, true);
            assert!(w > prev, "step {k}");
            assert!(w <= osc_env_dsp_word(k as f32, false) + 1e-6);
            prev = w;
        }
    }
}
