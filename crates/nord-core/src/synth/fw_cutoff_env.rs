//! Firmware composition of the filter envelope into the cutoff (opt-in `SynthLayer::fw_env_law`).
//!
//! The ORG core 2 pass adds the filter envelope to the cutoff in **semitones**: with the stored
//! knobs written the way the ARM edit path writes them (`fw_knob_laws.rs`), the oracle tap of the
//! stage coefficient `R = DM(0xC2925)` reads
//!
//! ```text
//! fc0 = 13.75 Hz * 2^(cutoff_knob / 12)          (knobs 0..=117)
//! fc  = fc0 * 2^(amount_knob * level / 12)       level = filter envelope chain, 0..1
//! R   = min(tan(pi fc / 2F), 1.99)               2F = 86,804.69 Hz, 4 samples per pass
//! ```
//!
//! i.e. one semitone per knob step at full level, 127 semitones (10.58 octaves) at full Filter
//! Env Amt. The default engine multiplies the Hz cutoff by `2^(amount * 8 * level)` instead
//! (`FILTER_ENV_OCTAVES`: 96 semitones at full amount, 31 short, and capped at 0.48 fs).
//!
//! Tags (`.agents/rules/provenance-in-code.md`): the law is VERIFIED against the SHARC oracle
//! (`docs/research/data/synth-cutoff-env-oracle.csv`: 17 base knobs 0..=114 within 0.6 % in `R`,
//! 8 amount x level points within 0.5 % in `R`, tests below); the stage rate `2F` is VERIFIED
//! through `fc0` against the panel table (13.75 Hz is `A0 / 2`, 440 Hz at knob 60). D236 settles
//! the level link: the engine `level` (filter chain times the velocity factor `F = G / 128`) is
//! VERIFIED against the oracle peak for 2 programs x 5 key bytes x 2 velocity settings
//! (`synth-env-level-link.csv`, test `level_link_rows_equal_the_oracle_peak`); key tracking adds
//! its semitones 1:1 to the same index sum (VERIFIED, D207 `m0`), so multiplying the Hz cutoff
//! first is equivalent below the `R` clip. Spec: `docs/specs/synth-engine.md` s4.3.
//!
//! D207 adds the other terms of the same index sum (`tools/synth_cutoff_terms.py`,
//! `docs/research/data/synth-cutoff-terms-oracle.csv`): the LFO term `512 * depth * lfo`
//! semitones (`fw_lfo_semitones`), the stage dependent map from the sum to `R`
//! (`fw_stage_r_of_knob`: exponential for every stage but the ladder, which reads the linear ramp),
//! and the facts that the filter drive byte and the key tracking offset add nothing else.

use super::SY_FRAME_RATE_HZ;

/// Stage sample rate of the filter: 4 samples per pass of 2 SY frames (`2 F`).
///
/// VERIFIED: with it `fc0 = 2F atan(R) / pi` of the oracle base coefficient equals
/// `13.75 * 2^(knob / 12)` (test `base_coefficient_is_the_tan_of_the_semitone_cutoff`).
pub const FW_STAGE_RATE_HZ: f32 = 2.0 * SY_FRAME_RATE_HZ;
/// Clip of the cutoff coefficient.
///
/// VERIFIED: the oracle tap saturates at `0x3FFEB852` = 1.99 (amount knob 96 / 127 at level
/// 0.99, cutoff knob 127; `synth-cutoff-env-oracle.csv`).
pub const FW_R_CLIP: f32 = 1.99;
/// Semitones of the cutoff per amount knob step at full envelope level.
///
/// VERIFIED: 8 oracle points, `R` within 0.5 % (test `env_rows_follow_one_semitone_per_knob_step`).
pub const FW_ENV_SEMITONES_PER_KNOB: f32 = 1.0;
/// Cutoff at knob 0, `A0 / 2`: VERIFIED (17 base knobs, `R` within 0.6 %).
pub const FW_CUTOFF_KNOB0_HZ: f32 = 13.75;

/// Semitones of the cutoff at full filter envelope amount and level (`w7 = 0x7F000000`).
///
/// VERIFIED: the envelope term is `128 * w7 * F-4` index units with `w7 = 127/128` (image default
/// of strip word 7) and `F-4 = level * amount` (float): 6 oracle rows, `w7` and `F-4` varied, within
/// 0.01 semitone (`synth-cutoff-terms-oracle.csv`, kind `env`, tests below).
pub const FW_ENV_FULL_SEMITONES: f32 = 127.0;
/// Semitones per unit product of the Q31 LFO depth word and the Q31 LFO word.
///
/// VERIFIED: `(w5 + m1) * lfo << 2` in index units of `2^24`, 12 oracle rows within 0.01 semitone
/// (kind `lfo`); the depth is the strip word 5 steady state, the LFO word `DM(0xC1E37)`.
pub const FW_LFO_SEMITONES_PER_UNIT: f32 = 512.0;
/// Depth word of `kLFOAmt = 127` with the destination on the cutoff, as a Q31 fraction (0.5).
///
/// VERIFIED: the ARM writes `amt / 127 * 0x40000000` to common word 3 for amt 0, 32, 64, 127
/// (`synth-lfo-arm-words.csv`, `tools/synth_lfo_words.py`).
pub const FW_LFO_DEPTH_FULL: f32 = 0.5;
/// LFO stage output at full scale as a Q31 fraction.
///
/// ASSUMED (EMPIRICAL lower bound): the LFO stages `0x12539E` and `0x1253AE` settle at 0.49997
/// and `0x1253BA` at 0.25 in the oracle with no rate word; no running LFO was traced, so full
/// scale 0.5 is the largest value seen. Confirm with a clip of an LFO to cutoff program.
pub const FW_LFO_OUTPUT_FULL: f32 = 0.5;

/// `R = tan(pi fc / 2F)` of a cutoff in Hz.
#[inline]
pub fn fw_r_of_hz(hz: f32) -> f32 {
    (std::f32::consts::PI * hz / FW_STAGE_RATE_HZ).tan()
}

/// Cutoff in Hz of a coefficient `R` (`2F atan(R) / pi`).
#[inline]
pub fn fw_hz_of_r(r: f32) -> f32 {
    FW_STAGE_RATE_HZ * r.atan() / std::f32::consts::PI
}

/// Cutoff of a stored cutoff knob (`13.75 * 2^(knob / 12)`, knobs 0..=117).
#[inline]
pub fn fw_cutoff_hz_of_knob(knob: f32) -> f32 {
    FW_CUTOFF_KNOB0_HZ * (knob / 12.0).exp2()
}

/// Cutoff in Hz after the filter envelope: `hz 2^(amount_knob level / 12)`, capped at the `R` clip.
///
/// `amount` is the layer's Filter Env Amt `-1..=1` (knob `/ 127`), `level` the envelope level.
/// Zero amount or level returns `hz` unchanged.
#[inline]
pub fn fw_env_cutoff_hz(hz: f32, amount: f32, level: f32) -> f32 {
    if amount == 0.0 || level <= 0.0 {
        return hz;
    }
    let knob = amount.clamp(-1.0, 1.0) * FW_ENV_FULL_SEMITONES;
    let semitones = FW_ENV_SEMITONES_PER_KNOB * knob * level;
    (hz * (semitones / 12.0).exp2()).min(fw_hz_of_r(FW_R_CLIP))
}

/// Semitones the LFO adds to the cutoff for a `kLFOAmt` knob `amount` (0..1) and an LFO value
/// `lfo` of the native engine (-1..1): `512 * (0.5 amount) * (0.5 lfo) = 128 amount lfo`.
#[inline]
pub fn fw_lfo_semitones(amount: f32, lfo: f32) -> f32 {
    FW_LFO_SEMITONES_PER_UNIT
        * (FW_LFO_DEPTH_FULL * amount.clamp(0.0, 1.0))
        * (FW_LFO_OUTPUT_FULL * lfo.clamp(-1.0, 1.0))
}

/// Cutoff in Hz after the LFO term, capped at the `R` clip.
#[inline]
pub fn fw_lfo_cutoff_hz(hz: f32, amount: f32, lfo: f32) -> f32 {
    (hz * (fw_lfo_semitones(amount, lfo) / 12.0).exp2()).min(fw_hz_of_r(FW_R_CLIP))
}

/// Oracle tap `R` of a cutoff index `knob` (semitone units, 13.75 Hz at 0).
///
/// VERIFIED (kind `r_law`): every stage but the ladder maps the index sum with the exponential
/// law (`tan(pi fc / 2F)`); the ladder stage `0x12549D` reads the linear ramp table
/// (`fw_strip_cutoff_r` at word `(knob - 52) << 24`) as its cutoff position, so its tap sits at
/// 0.59 for knob 81 where the exponential law gives 0.054. The two laws share the index sum.
pub fn fw_stage_r_of_knob(knob: f32, ladder: bool) -> f32 {
    if ladder {
        super::fw_strip_cutoff_r(((knob - 52.0) * (1 << 24) as f32).round() as i32)
    } else {
        fw_r_of_hz(fw_cutoff_hz_of_knob(knob)).min(FW_R_CLIP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/research/data/synth-cutoff-env-oracle.csv"
    ));

    /// `(kind, cutoff_knob, amount_knob, level, oracle_r)` rows of the oracle sweep.
    fn rows() -> Vec<(String, f32, f32, f32, f32)> {
        CSV.lines()
            .skip(1)
            .map(|l| {
                let f: Vec<&str> = l.split(',').collect();
                (
                    f[0].to_string(),
                    f[1].parse().unwrap(),
                    f[2].parse().unwrap(),
                    f[3].parse().unwrap(),
                    f[4].parse().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn base_coefficient_is_the_tan_of_the_semitone_cutoff() {
        let mut n = 0;
        for (kind, knob, _, _, r) in rows() {
            // Knobs above 117 use the steeper strip words of `fw_knob_laws::CUTOFF_TOP`.
            if kind != "base" || knob > 117.0 {
                continue;
            }
            let model = fw_r_of_hz(fw_cutoff_hz_of_knob(knob));
            assert!(
                (model / r - 1.0).abs() < 0.006,
                "knob {knob}: model {model}, oracle {r}"
            );
            n += 1;
        }
        assert!(n >= 17, "{n} base rows");
    }

    #[test]
    fn env_rows_follow_one_semitone_per_knob_step() {
        let mut checked = 0;
        for (kind, knob, amount, level, r) in rows() {
            if kind != "env" || amount == 0.0 || r >= FW_R_CLIP - 1e-3 {
                continue;
            }
            let fc = fw_env_cutoff_hz(fw_cutoff_hz_of_knob(knob), amount / 127.0, level);
            let model = fw_r_of_hz(fc);
            assert!(
                (model / r - 1.0).abs() < 0.005,
                "amount {amount} level {level}: model {model}, oracle {r}"
            );
            checked += 1;
        }
        assert!(checked >= 6, "{checked} rows below the clip");
    }

    #[test]
    fn env_rows_saturate_at_the_clip() {
        let sat: Vec<_> = rows()
            .into_iter()
            .filter(|r| r.0 == "env" && r.4 >= FW_R_CLIP - 1e-3)
            .collect();
        assert!(!sat.is_empty());
        for (_, knob, amount, level, _) in sat {
            let fc = fw_env_cutoff_hz(fw_cutoff_hz_of_knob(knob), amount / 127.0, level);
            assert!((fw_r_of_hz(fc) / FW_R_CLIP - 1.0).abs() < 1e-3, "{fc} Hz");
        }
    }

    #[test]
    fn panel_cutoff_matches_the_semitone_law_within_display_rounding() {
        for knob in 12..=114usize {
            let panel = crate::cbin::FILTER_CUTOFF_HZ_TABLE[knob];
            let law = fw_cutoff_hz_of_knob(knob as f32);
            assert!(
                (panel / law - 1.0).abs() < 0.05,
                "knob {knob}: panel {panel}, law {law}"
            );
        }
    }

    #[test]
    fn hz_and_r_round_trip_and_zero_amount_is_identity() {
        for hz in [100.0f32, 440.0, 2_000.0, 9_000.0] {
            assert!((fw_hz_of_r(fw_r_of_hz(hz)) / hz - 1.0).abs() < 1e-4);
        }
        assert_eq!(fw_env_cutoff_hz(370.0, 0.0, 1.0), 370.0);
        assert_eq!(fw_env_cutoff_hz(370.0, 0.5, 0.0), 370.0);
    }

    #[test]
    fn full_amount_sweeps_more_than_the_default_law() {
        let default_law = |hz: f32, a: f32, l: f32| hz * (a * 8.0 * l).exp2();
        let got = fw_env_cutoff_hz(200.0, 0.25, 1.0);
        assert!((got / 200.0 - (31.75f32 / 12.0).exp2()).abs() < 1e-3);
        assert!(got > default_law(200.0, 0.25, 1.0));
        let top = fw_env_cutoff_hz(5_000.0, 1.0, 1.0);
        assert!(
            (top - fw_hz_of_r(FW_R_CLIP)).abs() < 1.0,
            "capped at the R clip: {top}"
        );
    }

    const TERMS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/research/data/synth-cutoff-terms-oracle.csv"
    ));

    /// `(stage, inputs, oracle)` rows of one kind of the term measurement.
    fn term_rows(kind: &str) -> Vec<(String, String, f32)> {
        TERMS
            .lines()
            .skip(1)
            .filter_map(|l| {
                let f: Vec<&str> = l.split(',').collect();
                (f[0] == kind).then(|| (f[2].to_string(), f[3].to_string(), f[4].parse().unwrap()))
            })
            .collect()
    }

    fn field<'a>(inputs: &'a str, key: &str) -> &'a str {
        inputs
            .split(' ')
            .find_map(|p| p.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
            .unwrap_or_else(|| panic!("{key} in {inputs}"))
    }

    fn q31(word: &str) -> f32 {
        (u32::from_str_radix(word.trim_start_matches("0x"), 16).unwrap() as i32) as f32
            / 2_147_483_648.0
    }

    #[test]
    fn lfo_rows_follow_512_depth_lfo() {
        let rows = term_rows("lfo");
        assert_eq!(rows.len(), 12);
        for (_, inputs, oracle) in rows {
            let amt: f32 = field(&inputs, "amt").parse().unwrap();
            let lfo = q31(field(&inputs, "lfo")) / FW_LFO_OUTPUT_FULL;
            let got = fw_lfo_semitones(amt / 127.0, lfo);
            assert!(
                (got - oracle).abs() < 0.02,
                "{inputs}: {got} against {oracle}"
            );
        }
    }

    #[test]
    fn lfo_depth_word_is_the_arm_word_law() {
        // `synth-lfo-arm-words.csv`: dst 3 writes common word 3 = amt / 127 * 0x40000000.
        let csv = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/research/data/synth-lfo-arm-words.csv"
        ));
        let mut n = 0;
        for l in csv.lines().skip(1) {
            let f: Vec<&str> = l.split(',').collect();
            if f[0] == "3" && f[2] == "0xe02eb" {
                let amt: f64 = f[1].parse().unwrap();
                let word = u32::from_str_radix(f[3].trim_start_matches("0x"), 16).unwrap();
                let law = (amt / 127.0 * f64::from(0x4000_0000u32)).round();
                // the ARM float path keeps 24 significant bits: a few units of the low byte
                assert!((f64::from(word) - law).abs() <= 4.0, "amt {amt}: {word:#x}");
                n += 1;
            }
        }
        assert_eq!(n, 4);
        assert_eq!(fw_lfo_semitones(1.0, 1.0), 128.0);
        assert_eq!(fw_lfo_semitones(0.0, 1.0), 0.0);
    }

    #[test]
    fn env_rows_follow_127_semitones_per_unit_of_level_times_amount() {
        let mut n = 0;
        for (_, inputs, oracle) in term_rows("env") {
            if !inputs.starts_with("w7=0x7f000000") {
                continue;
            }
            let f4: f32 = field(&inputs, "F-4").parse().unwrap();
            assert!(
                (FW_ENV_FULL_SEMITONES * f4 - oracle).abs() < 0.02,
                "{inputs}: {oracle}"
            );
            n += 1;
        }
        assert_eq!(n, 2);
    }

    #[test]
    fn drive_byte_adds_nothing_and_key_offset_adds_its_semitones() {
        for (_, inputs, oracle) in term_rows("drive") {
            assert!(oracle.abs() < 1e-3, "{inputs}: drive moves R by {oracle}");
        }
        for (_, inputs, oracle) in term_rows("m0") {
            let m0: f32 = field(&inputs, "m0").parse().unwrap();
            assert!((m0 - oracle).abs() < 0.01, "{inputs}: {oracle}");
        }
    }

    #[test]
    fn stage_law_is_exponential_except_for_the_ladder_ramp() {
        let (mut ladder, mut exp) = (0, 0);
        for (name, inputs, oracle) in term_rows("r_law") {
            let is_ladder = name == "ladder";
            let knob: f32 = field(&inputs, "knob").parse().unwrap();
            let got = fw_stage_r_of_knob(knob, is_ladder);
            assert!(
                (got / oracle - 1.0).abs() < 0.006,
                "{name} {inputs}: {got} {oracle}"
            );
            if is_ladder {
                ladder += 1;
            } else {
                exp += 1;
            }
        }
        assert_eq!((ladder, exp), (4, 4));
        // key tracking moves the index sum, the ladder reads the same ramp (note offset -12).
        for (_, inputs, oracle) in term_rows("r_law_key") {
            let knob: f32 = field(&inputs, "knob").parse().unwrap();
            let got = fw_stage_r_of_knob(knob - 12.0, true);
            assert!(
                (got / oracle - 1.0).abs() < 0.006,
                "{inputs}: {got} {oracle}"
            );
        }
    }

    #[test]
    fn level_link_rows_equal_the_oracle_peak() {
        use crate::synth::fw_velocity::filter_factor;
        let csv = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/research/data/synth-env-level-link.csv"
        ));
        let mut n = 0;
        for l in csv.lines().skip(1) {
            // program,amount_knob,vel_setting,g,chain_peak,f4_peak,slot_f,index_before,index_peak,semitones_peak
            let f: Vec<&str> = l.split(',').collect();
            let amount: f32 = f[1].parse::<f32>().unwrap() / 127.0;
            let on = f[2] == "1";
            let g: u8 = f[3].parse().unwrap();
            let chain: f32 = f[4].parse().unwrap();
            let oracle: f32 = f[9].parse().unwrap();
            // engine: level = chain * fenv_peak, fenv_peak = F(g) with the setting on, else 1
            let level = chain * if on { filter_factor(g) } else { 1.0 };
            let base_hz = fw_cutoff_hz_of_knob(40.0);
            let semis = 12.0 * (fw_env_cutoff_hz(base_hz, amount, level) / base_hz).log2();
            assert!(
                (semis - oracle).abs() < 0.05,
                "{l}: {semis} against {oracle}"
            );
            n += 1;
        }
        assert!(n >= 16, "{n} rows");
    }

    #[test]
    fn lfo_cutoff_is_symmetric_in_semitones_and_capped() {
        let up = fw_lfo_cutoff_hz(1_000.0, 0.1, 1.0) / 1_000.0;
        let down = fw_lfo_cutoff_hz(1_000.0, 0.1, -1.0) / 1_000.0;
        assert!((up * down - 1.0).abs() < 1e-4, "{up} {down}");
        assert_eq!(fw_lfo_cutoff_hz(1_000.0, 0.0, 1.0), 1_000.0);
        let top = fw_lfo_cutoff_hz(10_000.0, 1.0, 1.0);
        assert!((top - fw_hz_of_r(FW_R_CLIP)).abs() < 1.0, "{top}");
    }
}
