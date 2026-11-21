//! Raw values of a decoded Synth layer that its engine-facing fields collapse (OQ31).
//!
//! Spec: `docs/specs/cbin-and-presets.md` section 5.8.
//!
//! Two things collapse in `SynthLayerDecoded`:
//!
//! * **Table indices.** The envelope time, cutoff and LFO rate tables are firmware display strings
//!   (`0.5 ms`, `11000 Hz`), so neighbouring steps share a value (`ENV_DECAY_MS_TABLE[102] ==
//!   [103]`, `FILTER_CUTOFF_HZ_TABLE[2] == [1]`). Going back from the value to an index returns
//!   the first step, so a file that stores the later one did not re-pack to its own bits.
//!   [`SynthLayerRaw`] keeps the stored index; [`hinted_idx`] uses it while the value still
//!   matches the table entry and falls back to the nearest entry once the value was edited.
//! * **Filter type.** `Edt_Syn_kFltType` has six values (`0` LP12, `1` LP24, `2` LP M, `3` LP+HP,
//!   `4` BP, `5` HP); the engine has no LP+HP model, so `3` decodes as `Lp12`.

use super::presets::table_val_to_idx;
use crate::synth::FilterModel;
use serde::{Deserialize, Serialize};

/// Stored raw values of one layer (all `None`-free: a layer built in code carries no raw set).
///
/// VERIFIED: every field is read from the payload bits named in its doc; decode + pack of all
/// 384 factory Synth presets reproduces the file (`ns4 preset-check <backup> --bits` lists no
/// collapsed run) and test `collapsed_synth_values_round_trip` repeats it on synthetic bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SynthLayerRaw {
    /// `Edt_Syn_kFltType` (sound `+218`, 3 bits): `0` LP12, `1` LP24, `2` LP M, `3` LP+HP, `4` BP,
    /// `5` HP (`6`, `7` unused).
    pub filter_type: u8,
    /// `Edt_Syn_kFltFreq` base (sound `+221`, 7 bits): index into `FILTER_CUTOFF_HZ_TABLE`.
    pub cutoff_idx: u8,
    /// `Edt_Syn_kLFORateClkDiv` base (sound `+133`, 7 bits): index into `LFO_RATE_HZ_TABLE`.
    pub lfo_rate_idx: u8,
    /// `Edt_Syn_kOscEnvAtk` / `Dcy` / `Rls` (sound `+71`, `+78`, `+85`, 7 bits each).
    pub osc_env_idx: [u8; 3],
}

/// Filter model the engine uses for a raw `Edt_Syn_kFltType`.
///
/// ASSUMED: `3` (LP+HP) is approximated by `Lp12` because the engine has no parallel LP + HP
/// model; `6`, `7` (never stored) fall to `HighPass`. What would confirm it: a hardware
/// recording of an LP+HP preset.
pub fn filter_model_of_type(raw: u32) -> FilterModel {
    match raw {
        0 | 3 => FilterModel::Lp12,
        1 => FilterModel::Lp24,
        2 => FilterModel::Ladder24,
        4 => FilterModel::BandPass,
        _ => FilterModel::HighPass,
    }
}

/// Raw `Edt_Syn_kFltType` to write: the stored one while it still decodes to `model`, else the
/// canonical value of `model`.
pub fn filter_type_bits(model: FilterModel, hint: Option<u8>) -> u32 {
    if let Some(h) = hint {
        if filter_model_of_type(u32::from(h)) == model {
            return u32::from(h);
        }
    }
    match model {
        FilterModel::Lp12 => 0,
        FilterModel::Lp24 => 1,
        FilterModel::Ladder24 => 2,
        FilterModel::BandPass => 4,
        FilterModel::HighPass => 5,
    }
}

/// Table index for `val`: `hint` while `table[hint]` is still `val`, else the nearest entry.
pub fn hinted_idx(table: &[f32; 128], val: f32, hint: Option<u8>) -> u32 {
    if let Some(h) = hint {
        let t = table[usize::from(h).min(127)];
        if (t - val).abs() <= 1e-4 * t.abs() {
            return u32::from(h.min(127));
        }
    }
    table_val_to_idx(table, val)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbin::bitstream::BitWriter;
    use crate::cbin::presets::{ENV_DECAY_MS_TABLE, FILTER_CUTOFF_HZ_TABLE};
    use crate::cbin::synth_preset::SynthPresetDecoded;

    #[test]
    fn a_hint_wins_over_a_duplicate_table_step_until_the_value_changes() {
        // The decay table repeats 14000 ms at steps 102 and 103: the nearest entry is 102.
        assert_eq!(ENV_DECAY_MS_TABLE[102], ENV_DECAY_MS_TABLE[103]);
        let ms = ENV_DECAY_MS_TABLE[103];
        assert_eq!(table_val_to_idx(&ENV_DECAY_MS_TABLE, ms), 102);
        assert_eq!(hinted_idx(&ENV_DECAY_MS_TABLE, ms, Some(103)), 103);
        assert_eq!(hinted_idx(&ENV_DECAY_MS_TABLE, ms, None), 102);
        let edited = ENV_DECAY_MS_TABLE[50];
        assert_eq!(hinted_idx(&ENV_DECAY_MS_TABLE, edited, Some(103)), 50);
        assert_eq!(FILTER_CUTOFF_HZ_TABLE[1], FILTER_CUTOFF_HZ_TABLE[2]);
        assert_eq!(
            hinted_idx(&FILTER_CUTOFF_HZ_TABLE, FILTER_CUTOFF_HZ_TABLE[2], Some(2)),
            2
        );
    }

    /// Writes `value` (`bits` wide) at payload bit `pos`.
    fn put(buf: &mut [u8], pos: usize, bits: usize, value: u32) {
        let mut w = BitWriter::new(buf);
        w.set_bit_pos(pos).unwrap();
        w.write_bits(value, bits).unwrap();
    }

    #[test]
    fn osc_env_amount_morph_words_decode_as_arm_depths() {
        // FM Harpies layer A and C: base 21 / 30 steps, wheel +2 / +10 (ARM cells
        // `[21, 2, 0, 0]`, `[30, 10, 0, 0]`, `docs/research/data/morph-fw-osc-env.csv`); the
        // three 8 bit words at `+99 / +107 / +115` store `depth + 120`.
        let mut bytes = SynthPresetDecoded::default().pack().unwrap();
        for (sound, base, wheel, at) in [(1472usize, 21u32, 2u32, 0u32), (2240, 30, 10, 0)] {
            put(&mut bytes, sound + 92, 7, base);
            put(&mut bytes, sound + 99, 8, 120 + wheel);
            put(&mut bytes, sound + 107, 8, 120 + at);
        }
        put(&mut bytes, 1856 + 107, 8, 120 - 35); // layer B, A.T. -35
        let dec = SynthPresetDecoded::unpack(&bytes).unwrap();
        let steps =
            |m: crate::cbin::MorphDeltas| [m.wheel, m.aftertouch, m.ctrl_pedal].map(|d| d * 120.0);
        assert_eq!(steps(dec.layer_a.osc_env_amount_morph), [2.0, 0.0, 0.0]);
        assert_eq!(steps(dec.layer_c.osc_env_amount_morph), [10.0, 0.0, 0.0]);
        assert_eq!(steps(dec.layer_b.osc_env_amount_morph), [0.0, -35.0, 0.0]);
        assert!((dec.layer_a.osc_env_amount - 21.0 / 60.0).abs() < 1e-6);
        assert_eq!(dec.pack().unwrap(), bytes, "decode + pack must be lossless");
        // an edited depth is written back and decodes again
        let mut edited = dec.clone();
        edited.layer_a.osc_env_amount_morph.ctrl_pedal = -45.0 / 120.0;
        let again = SynthPresetDecoded::unpack(&edited.pack().unwrap()).unwrap();
        assert_eq!(steps(again.layer_a.osc_env_amount_morph), [2.0, 0.0, -45.0]);
    }

    #[test]
    fn collapsed_synth_values_round_trip() {
        let mut bytes = SynthPresetDecoded::default().pack().unwrap();
        // Sound A (`1472`), sound B (`1856`), sound C (`2240`): every collapsed cluster of the
        // factory backup, as raw bits.
        for (i, sound) in [1472usize, 1856, 2240].into_iter().enumerate() {
            put(&mut bytes, sound + 218, 3, 3); // filter type LP+HP
            put(&mut bytes, sound + 221, 7, 2); // cutoff step 2 (== step 1)
            put(&mut bytes, sound + 133, 7, 2); // LFO rate step 2 (== steps 0, 1)
            put(&mut bytes, sound + 78, 7, 98); // osc env decay step 98 (== 97)
            put(&mut bytes, sound + 202, 7, 103); // amp decay step 103 (== 102)
            put(&mut bytes, sound + 325, 7, 100); // filter decay step 100 (== 99)
            put(&mut bytes, sound + 171, 8, 128 + i as u32); // LFO amount morph, Mod Wheel
            put(&mut bytes, sound + 294, 8, 140); // Filter Env Amount morph, Mod Wheel
            put(&mut bytes, sound + 140, 8, 90); // LFO rate morph, Mod Wheel
        }
        let dec = SynthPresetDecoded::unpack(&bytes).unwrap();
        assert_eq!(dec.pack().unwrap(), bytes, "decode + pack must be lossless");
        let l = &dec.layer_c;
        assert_eq!(l.raw.unwrap().filter_type, 3);
        assert_eq!(l.filter_model, FilterModel::Lp12);
        assert!(l.lfo_amount_morph.wheel > 0.0 && l.filter_env_amount_morph.wheel > 0.0);
        assert!(l.lfo_rate_morph.wheel < 0.0);

        // An edit wins over the stored raw value.
        let mut edited = dec.clone();
        edited.layer_a.filter_model = FilterModel::Lp24;
        edited.layer_a.filter_cutoff_hz = FILTER_CUTOFF_HZ_TABLE[50];
        let out = edited.pack().unwrap();
        let back = SynthPresetDecoded::unpack(&out).unwrap();
        assert_eq!(back.layer_a.filter_model, FilterModel::Lp24);
        assert_eq!(back.layer_a.raw.unwrap().filter_type, 1);
        assert_eq!(back.layer_a.raw.unwrap().cutoff_idx, 50);
        assert_eq!(back.layer_b, dec.layer_b);
    }

    #[test]
    fn filter_type_three_is_lp_hp_and_keeps_its_bits_until_the_model_changes() {
        assert_eq!(filter_model_of_type(3), FilterModel::Lp12);
        assert_eq!(filter_type_bits(FilterModel::Lp12, Some(3)), 3);
        assert_eq!(filter_type_bits(FilterModel::Lp12, Some(0)), 0);
        assert_eq!(filter_type_bits(FilterModel::Lp12, None), 0);
        assert_eq!(filter_type_bits(FilterModel::Lp24, Some(3)), 1);
        assert_eq!(filter_type_bits(FilterModel::HighPass, Some(7)), 7);
        for raw in 0..6u32 {
            let m = filter_model_of_type(raw);
            assert_eq!(filter_type_bits(m, Some(raw as u8)), raw);
        }
    }
}
