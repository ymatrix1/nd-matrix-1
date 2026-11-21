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
//! through `fc0` against the panel table (13.75 Hz is `A0 / 2`, 440 Hz at knob 60). That the engine
//! `level` is the oracle chain level times the velocity peak, and that key tracking multiplies
//! the Hz cutoff before this law, are ASSUMED. Spec: `docs/specs/synth-engine.md` s4.3.

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
    let knob = amount.clamp(-1.0, 1.0) * 127.0;
    let semitones = FW_ENV_SEMITONES_PER_KNOB * knob * level;
    (hz * (semitones / 12.0).exp2()).min(fw_hz_of_r(FW_R_CLIP))
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
}
