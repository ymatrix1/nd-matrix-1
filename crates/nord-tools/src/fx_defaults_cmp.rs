//! Flip evidence for `--fx-defaults firmware` (D101, OQ34): for each of the differing FX default
//! words, the chain rendered with the current engine default and with the firmware default is
//! compared with the firmware model (the port, oracle checked word for word, `fx-vs-oracle.md`)
//! driven by the firmware word. Numbers only (`docs/research/fx-defaults-flip.md`).
//!
//! Each word is varied alone: every other word of the case stays at its firmware default, so the
//! verdict of a word does not depend on another flip.
use crate::fx_cases::{bands_db, render, rms_db, stimulus, Case, FS};
use crate::fx_parity::{aligned_null_db, chain_with};
use nord_core::cbin::FxDecoded;
use nord_core::fx::fw_delay_time::delay_index;
use nord_core::fx::{FirmwareFx, LayerFxChain};

/// Frames per render (a power of two, 1.5 s at the DSP rate).
/// ASSUMED: tool convention, long enough for the Mod LFOs and the delay repeats to show.
pub const FRAMES: usize = 1 << 16;
/// Stimuli of every word.
pub const STIMS: [&str; 3] = ["noise", "notes", "impulse"];
/// Band error below which two chains count as the same (dB, rms over the octave bands).
/// ASSUMED: 0.1 dB is far below the 1 dB just-noticeable level change; a tie flips nothing.
pub const TIE_DB: f64 = 0.1;

/// The words under test: `(field, block)`; the field names are those of `FW_DEFAULTS`.
pub const WORDS: [&str; 8] = [
    "mod1_amount",
    "mod2_amount",
    "eq_mid_freq_hz",
    "comp_amount",
    "delay_time_ms",
    "delay_mix",
    "delay_feedback",
    "reverb_mix",
];

/// One variant of a word: a block configuration the word is rendered in.
#[derive(Clone, Debug)]
pub struct Variant {
    pub word: &'static str,
    pub label: String,
    /// Sets the word (`value`, in the chain's own unit) on top of the firmware defaults.
    pub setup: fn(&mut FxDecoded, f32),
    /// Chain unit values `(engine default, firmware default)`.
    pub values: (f32, f32),
}

/// A decoded FX block with every block off and every word at its firmware default.
pub fn base_decoded(fw: &LayerFxChain) -> FxDecoded {
    let mut d = FxDecoded::default();
    d.mod1_enabled = false;
    d.mod2_enabled = false;
    d.amp_enabled = false;
    d.comp_enabled = false;
    d.delay_enabled = false;
    d.reverb_enabled = false;
    d.mod1_rate = 0.503_937;
    d.mod2_rate = 0.503_937;
    d.mod1_amount = fw.mod1_amount;
    d.mod2_amount = fw.mod2_amount;
    d.comp_amount = fw.comp_amount;
    d.delay_mix = fw.delay_mix;
    d.delay_feedback = fw.delay_feedback;
    d.delay_rate_norm = delay_norm_for_ms(fw.delay_time_ms);
    d.reverb_amount = fw.reverb_mix;
    d
}

/// The `kDlyRateClkDiv / 16383` fraction whose firmware time is closest to `ms`.
pub fn delay_norm_for_ms(ms: f32) -> f32 {
    let best = (0..=16_383u16)
        .min_by(|a, b| {
            let (da, db) = (
                (delay_index(*a) - f64::from(ms)).abs(),
                (delay_index(*b) - f64::from(ms)).abs(),
            );
            da.total_cmp(&db)
        })
        .unwrap_or(0);
    f32::from(best) / 16_383.0
}

/// The engine defaults and the firmware defaults of a fresh chain.
pub fn default_chains() -> (LayerFxChain, LayerFxChain) {
    let engine = LayerFxChain::new(FS);
    let mut fw = LayerFxChain::new(FS);
    fw.apply_firmware_defaults();
    (engine, fw)
}

fn field(c: &LayerFxChain, name: &str) -> f32 {
    match name {
        "mod1_amount" => c.mod1_amount,
        "mod2_amount" => c.mod2_amount,
        "eq_mid_freq_hz" => c.eq_mid_freq_hz,
        "comp_amount" => c.comp_amount,
        "delay_time_ms" => c.delay_time_ms,
        "delay_mix" => c.delay_mix,
        "delay_feedback" => c.delay_feedback,
        "reverb_mix" => c.reverb_mix,
        other => panic!("unknown word {other}"),
    }
}

/// Every variant: Mod 1 types 0 and 1 (the boot types), Mod 2 types 0 (firmware default) and 4
/// (engine default), the compressor, the three delay words and the reverb at types 2, 5, 7.
pub fn variants() -> Vec<Variant> {
    let (e, f) = default_chains();
    let v = |word: &'static str, label: &str, setup: fn(&mut FxDecoded, f32)| Variant {
        word,
        label: label.to_string(),
        setup,
        values: (field(&e, word), field(&f, word)),
    };
    let mut out = Vec::new();
    for ty in 0..2u8 {
        out.push(v(
            "mod1_amount",
            &format!("type{ty}"),
            match ty {
                0 => |d, a| (d.mod1_enabled, d.mod1_type_raw, d.mod1_amount) = (true, 0, a),
                _ => |d, a| (d.mod1_enabled, d.mod1_type_raw, d.mod1_amount) = (true, 1, a),
            },
        ));
    }
    out.push(v("mod2_amount", "type0-PHAS", |d, a| {
        (d.mod2_enabled, d.mod2_type_raw, d.mod2_amount) = (true, 0, a);
    }));
    out.push(v("mod2_amount", "type4-CHOR", |d, a| {
        (d.mod2_enabled, d.mod2_type_raw, d.mod2_amount) = (true, 4, a);
    }));
    out.push(v("comp_amount", "slow", |d, a| {
        (d.comp_enabled, d.comp_amount) = (true, a);
    }));
    out.push(v("delay_time_ms", "mix-fw", |d, ms| {
        d.delay_enabled = true;
        d.delay_rate_norm = delay_norm_for_ms(ms);
    }));
    out.push(v("delay_mix", "time-fw", |d, a| {
        (d.delay_enabled, d.delay_mix) = (true, a);
    }));
    out.push(v("delay_feedback", "time-fw", |d, a| {
        (d.delay_enabled, d.delay_feedback) = (true, a);
    }));
    for ty in [2u8, 5, 7] {
        out.push(v(
            "reverb_mix",
            &format!("type{ty}"),
            match ty {
                2 => |d, a| set_reverb(d, 2, a),
                5 => |d, a| set_reverb(d, 5, a),
                _ => |d, a| set_reverb(d, 7, a),
            },
        ));
    }
    out
}

fn set_reverb(d: &mut FxDecoded, ty: u8, a: f32) {
    d.reverb_enabled = true;
    d.reverb_type_raw = ty >> 1;
    d.amp_type_new = u16::from(ty & 1) << 8;
    d.reverb_amount = a;
}

/// Measurements of one chain against the firmware model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Score {
    /// Chain rms minus model rms (dB).
    pub rms_delta_db: f64,
    /// Rms over the octave bands of the band gain difference (dB).
    pub band_err_db: f64,
    /// Aligned null depth (dB re the model).
    pub null_db: f64,
}

/// Rms of the octave band gain differences of `y` and the model `m` on the input `x`.
pub fn band_err(x: &[f32], y: &[f32], m: &[f32]) -> f64 {
    let (a, b) = (bands_db(x, y), bands_db(x, m));
    // Bands with no input energy (an impulse has none in some bands) read NaN: skipped.
    let d: Vec<f64> = a
        .iter()
        .zip(&b)
        .map(|(p, q)| p - q)
        .filter(|v| v.is_finite())
        .collect();
    if d.is_empty() {
        return f64::NAN;
    }
    (d.iter().map(|v| v * v).sum::<f64>() / d.len() as f64).sqrt()
}

/// Score of `y` against the model output `m` on the stimulus `x`.
pub fn score(x: &[f32], y: &[f32], m: &[f32]) -> Score {
    Score {
        rms_delta_db: rms_db(y) - rms_db(m),
        band_err_db: band_err(x, y, m),
        null_db: aligned_null_db(y, m),
    }
}

/// Which default is closer to the firmware model: by the band error, by the null depth when a
/// band error does not exist (impulse stimulus; the lower null wins, a tie under 1 dB).
pub fn verdict(engine: &Score, fw: &Score) -> &'static str {
    if !(engine.band_err_db.is_finite() && fw.band_err_db.is_finite()) {
        let d = fw.null_db - engine.null_db;
        return if d.abs() < 1.0 {
            "tie"
        } else if d < 0.0 {
            "firmware"
        } else {
            "engine"
        };
    }
    let d = engine.band_err_db - fw.band_err_db;
    if d.abs() < TIE_DB {
        "tie"
    } else if d > 0.0 {
        "firmware"
    } else {
        "engine"
    }
}

/// One output row of a variant on a stimulus: `(engine score, firmware score)`.
pub fn run(var: &Variant, stim: &str, n: usize) -> (Score, Score) {
    let (_, f) = default_chains();
    let (x, _) = stimulus(stim, n);
    let make = |value: f32, fx: FirmwareFx| {
        let mut dec = base_decoded(&f);
        (var.setup)(&mut dec, value);
        let case = Case {
            family: "defaults",
            label: var.label.clone(),
            dec,
            port: "",
        };
        render(&mut chain_with(&case, fx), &x)
    };
    let model = make(var.values.1, FirmwareFx::ALL);
    let engine = make(var.values.0, FirmwareFx::DEFAULT);
    let fw = make(var.values.1, FirmwareFx::DEFAULT);
    (score(&x, &engine, &model), score(&x, &fw, &model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_norm_hits_the_firmware_times() {
        for ms in [250.0f32, 375.0] {
            let raw = (delay_norm_for_ms(ms) * 16_383.0).round() as u16;
            assert!((delay_index(raw) - f64::from(ms)).abs() < 2.0, "{ms}");
        }
    }

    #[test]
    fn variants_cover_every_word_once_or_more() {
        let v = variants();
        for w in WORDS {
            if w == "eq_mid_freq_hz" {
                continue;
            }
            assert!(v.iter().any(|x| x.word == w), "{w}");
        }
        assert!(v.iter().all(|x| x.values.0 != x.values.1));
    }

    #[test]
    fn firmware_value_scores_a_perfect_null_for_a_ported_word() {
        let v = variants()
            .into_iter()
            .find(|v| v.word == "mod2_amount" && v.label == "type0-PHAS")
            .unwrap();
        let (engine, fw) = run(&v, "noise", 4096);
        assert!(fw.null_db < -100.0, "{fw:?}");
        assert!(engine.null_db > fw.null_db + 20.0, "{engine:?}");
        assert_eq!(verdict(&engine, &fw), "firmware");
    }

    #[test]
    fn verdict_names_the_closer_default_and_ties() {
        let s = |b| Score {
            rms_delta_db: 0.0,
            band_err_db: b,
            null_db: 0.0,
        };
        assert_eq!(verdict(&s(3.0), &s(1.0)), "firmware");
        assert_eq!(verdict(&s(1.0), &s(3.0)), "engine");
        assert_eq!(verdict(&s(1.0), &s(1.05)), "tie");
    }

    #[test]
    fn verdict_falls_back_to_the_null_depth_without_a_band_error() {
        let s = |n| Score {
            rms_delta_db: 0.0,
            band_err_db: f64::NAN,
            null_db: n,
        };
        assert_eq!(verdict(&s(-20.0), &s(-300.0)), "firmware");
        assert_eq!(verdict(&s(-300.0), &s(-20.0)), "engine");
        assert_eq!(verdict(&s(-20.0), &s(-20.5)), "tie");
    }

    #[test]
    fn score_of_the_model_against_itself_is_zero() {
        let (x, _) = stimulus("noise", 4096);
        let s = score(&x, &x, &x);
        assert!(s.band_err_db < 1e-9 && s.rms_delta_db.abs() < 1e-9 && s.null_db < -200.0);
    }
}
