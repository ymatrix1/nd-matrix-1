//! Oracle envelope law of the ORG core 2 amp chain (opt-in `SynthLayer::fw_adr_law`, D205).
//!
//! Two facts measured with the SHARC oracle (`tools/synth_env_law.py`, one run per row: the ARM
//! rate words injected, the note keyed with the captured key word whose byte 1 is `0x80 + g`,
//! the three envelope chains of slot 0 tapped every pass):
//!
//! * **Amp attack velocity scaling.** For a negative attack index `w` the amp chain's attack
//!   record uses the table entry of `floor(w * G / 128)`, `G` the effective key byte
//!   ([`super::fw_velocity::effective_g`]: `g`, and 128 for `g = 127`). Positive words, the
//!   filter chain, the oscillator chain, the decay and the release are not scaled. A harder key
//!   (larger `g`) therefore attacks faster; a soft key attacks up to 128 / `G` slower in index.
//! * **Decay sustain level.** The decay record of every chain stops at `thr = 0.5`; the value is
//!   the image default (`DM(0xC0DCE`), never written by the ARM in the recall or key streams) and
//!   does not depend on the rate word, `g` or the key.
//!
//! Tags (`.agents/rules/provenance-in-code.md`): both facts are VERIFIED against the oracle (see
//! each item). That the hardware keeps the 0.5 plateau on the audio output (the layer gain stage
//! after the chain is not isolated) is ASSUMED. Spec: `docs/specs/synth-engine.md` s4.8.

use super::fw_knob_laws::rate_index_for_knob;
use super::fw_velocity::effective_g;
use super::strip_map::attack_index;
use super::{StripVoice, SynthLayer};

/// Settled attack coefficient `b` per pass of the ADR attack record for the indexes -64..=63
/// (array position `index + 64`), the slope of the oracle ramp measured at every integer index.
///
/// VERIFIED: `docs/research/data/synth-env-attack-table.csv` (filter chain of slot 0, key velocity
/// `g = 127`, `python tools/synth_env_law.py grid`), 6 significant digits, test
/// `attack_table_equals_the_oracle_csv`. The older `fw_env_rates.rs` table is the same sweep read
/// 3 passes after each poke, before the installer rewrote the coefficient (every 16th pass): it
/// holds one value over 5 or 6 indexes and is up to 2.1 times too high in between (D205).
const ATTACK_B: [f32; 128] = [
    9.216160e-2,
    7.570280e-2,
    6.247760e-2,
    5.179540e-2,
    4.312440e-2,
    3.605260e-2,
    3.025890e-2,
    2.549180e-2,
    2.155310e-2,
    1.828590e-2,
    1.556550e-2,
    1.329190e-2,
    1.138520e-2,
    9.780710e-3,
    8.426150e-3,
    7.279020e-3,
    6.304610e-3,
    5.474500e-3,
    4.765350e-3,
    4.157880e-3,
    3.636150e-3,
    3.186920e-3,
    2.799170e-3,
    2.463690e-3,
    2.172780e-3,
    1.919940e-3,
    1.699720e-3,
    1.507510e-3,
    1.339420e-3,
    1.192110e-3,
    1.062790e-3,
    9.490300e-4,
    8.487900e-4,
    7.603000e-4,
    6.820600e-4,
    6.127600e-4,
    5.512800e-4,
    4.966600e-4,
    4.480500e-4,
    4.047200e-4,
    3.660500e-4,
    3.314900e-4,
    3.005600e-4,
    2.728400e-4,
    2.479600e-4,
    2.256000e-4,
    2.054900e-4,
    1.873700e-4,
    1.710300e-4,
    1.562700e-4,
    1.429300e-4,
    1.308600e-4,
    1.199200e-4,
    1.100100e-4,
    1.010000e-4,
    9.282000e-5,
    8.537000e-5,
    7.859000e-5,
    7.241000e-5,
    6.677000e-5,
    6.162000e-5,
    5.691000e-5,
    5.260000e-5,
    4.866000e-5,
    4.505000e-5,
    4.173000e-5,
    3.869000e-5,
    3.589000e-5,
    3.332000e-5,
    3.096000e-5,
    2.878000e-5,
    2.677000e-5,
    2.492000e-5,
    2.321000e-5,
    2.164000e-5,
    2.018000e-5,
    1.883000e-5,
    1.758000e-5,
    1.643000e-5,
    1.536000e-5,
    1.436000e-5,
    1.344000e-5,
    1.259000e-5,
    1.179000e-5,
    1.105000e-5,
    1.037000e-5,
    9.730000e-6,
    9.130000e-6,
    8.580000e-6,
    8.060000e-6,
    7.580000e-6,
    7.130000e-6,
    6.710000e-6,
    6.310000e-6,
    5.950000e-6,
    5.600000e-6,
    5.280000e-6,
    4.980000e-6,
    4.700000e-6,
    4.440000e-6,
    4.190000e-6,
    3.960000e-6,
    3.740000e-6,
    3.540000e-6,
    3.350000e-6,
    3.170000e-6,
    3.000000e-6,
    2.840000e-6,
    2.690000e-6,
    2.550000e-6,
    2.410000e-6,
    2.290000e-6,
    2.170000e-6,
    2.060000e-6,
    1.960000e-6,
    1.860000e-6,
    1.770000e-6,
    1.680000e-6,
    1.590000e-6,
    1.520000e-6,
    1.440000e-6,
    1.370000e-6,
    1.310000e-6,
    1.240000e-6,
    1.180000e-6,
    1.130000e-6,
    1.070000e-6,
    1.020000e-6,
];

/// Settled attack coefficient of an index (clamped to -64..=63).
#[inline]
pub fn attack_b(index: i32) -> f32 {
    ATTACK_B[(index.clamp(-64, 63) + 64) as usize]
}

/// Sustain level of the decay record (both chains).
///
/// VERIFIED: oracle `level_at_off` = 0.5000 (4 digits) for decay words -64..=0 in both the amp and
/// the filter chain, `g` 127 and 64, key 60 (slower words are still falling at key off) (`docs/research/data/synth-env-law-sustain.csv`,
/// `python tools/synth_env_law.py grid`); the DSP image holds it at `DM(0xC0DCE)` and the five
/// recall + key captures never write that address (`docs/research/synth-env-law.md`).
pub const FW_DECAY_SUSTAIN: f32 = 0.5;

/// Effective attack index of the amp chain: `floor(w * G / 128)` for `w < 0`, `w` otherwise.
///
/// VERIFIED: 256 of 256 rows (8 key bytes `g` 10..=110 x 32 negative words -64..=-2, key 60)
/// select exactly this table entry; the alternatives `g / 127` with trunc, round, floor or ceil
/// reach 78, 192, 178 and 78 (`tools/synth_env_law.py fit`, test `law_matches_the_oracle_rows`).
/// 77 of 77 rows with a positive word or `g = 127` are unscaled.
#[inline]
pub fn amp_attack_index(index: i32, g: u8) -> i32 {
    if index >= 0 {
        index
    } else {
        // Arithmetic shift is the floor of the division by 128.
        (index * effective_g(g) as i32) >> 7
    }
}

impl SynthLayer {
    /// Amp attack index the chain uses for a new voice (stored rate knob, else the panel time).
    pub(super) fn amp_attack_word(&self) -> i32 {
        self.active_knobs().map_or_else(
            || attack_index(self.attack_sec),
            |k| rate_index_for_knob(k.amp.attack),
        )
    }

    /// Filter attack index of the chain (stored rate knob, else the panel time).
    pub(super) fn filter_attack_word(&self) -> i32 {
        self.active_knobs().map_or_else(
            || attack_index(self.filter_env_attack_sec),
            |k| rate_index_for_knob(k.filter.attack),
        )
    }

    /// Strip state of a new voice. With `fw_adr_law` the amp attack record is the table entry of
    /// the velocity scaled index (`fw_adr_law.rs`); otherwise the shared record.
    pub(super) fn new_strip_voice(&self, key_byte: u8) -> StripVoice {
        if self.fw_adr_law {
            StripVoice::with_scaled_amp_attack(self.amp_attack_word(), key_byte)
        } else {
            StripVoice::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = include_str!("../../../../docs/research/data/synth-env-law-attack.csv");

    #[test]
    fn law_matches_the_oracle_rows() {
        let mut n = 0;
        for l in CSV.lines().skip(1) {
            let c: Vec<&str> = l.trim().split(',').collect();
            let (g, w, idx): (u8, i32, i32) = (
                c[0].parse().unwrap(),
                c[1].parse().unwrap(),
                c[2].parse().unwrap(),
            );
            assert_eq!(amp_attack_index(w, g), idx, "g {g} word {w}");
            n += 1;
        }
        assert!(n >= 300, "{n} rows");
    }

    #[test]
    fn full_velocity_and_positive_words_are_unscaled() {
        for w in -64..=63 {
            assert_eq!(amp_attack_index(w, 127), w);
            assert_eq!(amp_attack_index(w, 128), w);
        }
        for g in [1u8, 10, 64, 98, 126] {
            assert_eq!(amp_attack_index(0, g), 0);
            assert_eq!(amp_attack_index(30, g), 30);
        }
    }

    #[test]
    fn soft_keys_attack_slower_and_the_floor_goes_down() {
        assert_eq!(amp_attack_index(-64, 10), -5);
        assert_eq!(amp_attack_index(-2, 10), -1, "floor, not trunc");
        assert_eq!(amp_attack_index(-60, 64), -30);
        let mut last = 1;
        for g in 1..=127u8 {
            let i = amp_attack_index(-40, g);
            assert!(i <= 0 && i <= last, "g {g}: {i} after {last}");
            last = i;
        }
        assert_eq!(amp_attack_index(-40, 127), -40);
    }

    #[test]
    fn attack_table_equals_the_oracle_csv() {
        let rows = include_str!("../../../../docs/research/data/synth-env-attack-table.csv");
        let mut n = 0;
        for l in rows.lines().skip(1) {
            let (i, v) = l.trim().split_once(',').unwrap();
            let (i, v): (i32, f32) = (i.parse().unwrap(), v.parse().unwrap());
            assert!((attack_b(i) / v - 1.0).abs() < 1e-5, "index {i}");
            n += 1;
        }
        assert_eq!(n, 128);
    }

    #[test]
    fn attack_table_falls_with_the_index_and_beats_the_plateaus_in_between() {
        for i in -64..63 {
            assert!(attack_b(i + 1) < attack_b(i), "index {i}");
        }
        // the older plateau table holds 0x3dbcbf3f = 0.09216 from -64 to -59.
        assert!(attack_b(-60) < 0.5 * 0.092_16);
        assert_eq!(attack_b(-100), attack_b(-64));
        assert_eq!(attack_b(100), attack_b(63));
    }

    #[test]
    fn law_holds_for_the_keys_36_and_84() {
        let rows = include_str!("../../../../docs/research/data/synth-env-law-keys.csv");
        let mut n = 0;
        for l in rows.lines().skip(1) {
            let c: Vec<i32> = l.trim().split(',').map(|x| x.parse().unwrap()).collect();
            assert_eq!(
                amp_attack_index(c[2], c[1] as u8),
                c[3],
                "key {} g {}",
                c[0],
                c[1]
            );
            n += 1;
        }
        assert_eq!(n, 80);
    }

    #[test]
    fn sustain_rows_settle_at_half() {
        let rows = include_str!("../../../../docs/research/data/synth-env-law-sustain.csv");
        let mut n = 0;
        for l in rows.lines().skip(1) {
            let c: Vec<&str> = l.trim().split(',').collect();
            let (word, level): (i32, f32) = (c[1].parse().unwrap(), c[3].parse().unwrap());
            if word <= 0 {
                assert!((level - FW_DECAY_SUSTAIN).abs() < 1e-4, "{l}");
                n += 1;
            }
        }
        assert_eq!(n, 32);
    }
}
