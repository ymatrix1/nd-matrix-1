//! The block sync of the top-level parameters (layers, levels, organ, piano, synth), one function
//! per [`Group`](crate::program_hold::Group) so a loaded program can keep a group while the host
//! has not touched it (`program_hold.rs`, D215). Moved out of `params.rs` unchanged.

use crate::params::Stage4Params;
use nord_core::engine::Stage4Engine;
use nord_core::morph::LayerId;
use nord_core::organ::{OrganModel, RotarySpeed};
use nord_core::piano::{PianoCategory, PianoTimbre, VelocityTouch};
use nord_core::synth::SynthOscMode;

impl Stage4Params {
    fn set_mask_bit(mask: &mut u8, layer: LayerId, active: bool) {
        let bit = 1 << (layer as u8);
        if active {
            *mask |= bit;
        } else {
            *mask &= !bit;
        }
    }

    /// Scene switch, layer enables of the active scene, and the split points.
    pub(crate) fn sync_layers_and_splits(&self, engine: &mut Stage4Engine) {
        engine.scene.scene_2_active = self.layer_scene_ii.value();

        // Layer mask sync
        let target_mask = if self.layer_scene_ii.value() {
            &mut engine.scene.scene_2_mask
        } else {
            &mut engine.scene.scene_1_mask
        };
        Self::set_mask_bit(target_mask, LayerId::OrganA, self.organ_a_enabled.value());
        Self::set_mask_bit(target_mask, LayerId::OrganB, self.organ_b_enabled.value());
        Self::set_mask_bit(target_mask, LayerId::PianoA, self.piano_a_enabled.value());
        Self::set_mask_bit(target_mask, LayerId::PianoB, self.piano_b_enabled.value());
        Self::set_mask_bit(target_mask, LayerId::SynthA, self.synth_a_enabled.value());
        Self::set_mask_bit(target_mask, LayerId::SynthB, self.synth_b_enabled.value());
        Self::set_mask_bit(target_mask, LayerId::SynthC, self.synth_c_enabled.value());

        // Split enabled
        let split_on = self.split_enabled.value();
        let split_notes = [
            self.split_1_note.value() as u8,
            self.split_2_note.value() as u8,
            self.split_3_note.value() as u8,
        ];
        let split_crossfades = [
            self.split_1_crossfade.value(),
            self.split_2_crossfade.value(),
            self.split_3_crossfade.value(),
        ];
        for ((s, note), crossfade) in engine
            .split_router
            .splits
            .iter_mut()
            .zip(split_notes)
            .zip(split_crossfades)
        {
            s.enabled = split_on;
            s.midi_note = note;
            s.crossfade = match crossfade {
                1 => nord_core::morph::CrossfadeWidth::Small,
                2 => nord_core::morph::CrossfadeWidth::Large,
                _ => nord_core::morph::CrossfadeWidth::Off,
            };
        }
    }

    /// The master compressor and the seven layer levels, scaled by the master level.
    pub(crate) fn sync_levels(&self, engine: &mut Stage4Engine) {
        engine.master_bus.compressor_amount = self.master_compressor.value();
        let master_gain = self.master_level.value();
        engine.layer_levels[0].base = self.organ_a_level.value() * master_gain;
        engine.layer_levels[1].base = self.organ_b_level.value() * master_gain;
        engine.layer_levels[2].base = self.piano_a_level.value() * master_gain;
        engine.layer_levels[3].base = self.piano_b_level.value() * master_gain;
        engine.layer_levels[4].base = self.synth_a_level.value() * master_gain;
        engine.layer_levels[5].base = self.synth_b_level.value() * master_gain;
        engine.layer_levels[6].base = self.synth_c_level.value() * master_gain;
    }

    /// Organ model, drawbars, percussion and rotary.
    pub(crate) fn sync_organ(&self, engine: &mut Stage4Engine) {
        let organ_model = match self.organ_model.value() {
            1 => OrganModel::Vox,
            2 => OrganModel::Farfisa,
            3 => OrganModel::Pipe1,
            4 => OrganModel::Pipe2,
            _ => OrganModel::B3,
        };
        engine.organ_a.model = organ_model;
        engine.organ_b.model = organ_model;

        let drawbars = [
            self.drawbar_1_16.value() as u8,
            self.drawbar_2_5_1_3.value() as u8,
            self.drawbar_3_8.value() as u8,
            self.drawbar_4_4.value() as u8,
            self.drawbar_5_2_2_3.value() as u8,
            self.drawbar_6_2.value() as u8,
            self.drawbar_7_1_3_5.value() as u8,
            self.drawbar_8_1_1_3.value() as u8,
            self.drawbar_9_1.value() as u8,
        ];
        engine.organ_a.drawbars = drawbars;

        engine.organ_b.drawbars = [
            self.drawbar_b_1_16.value() as u8,
            self.drawbar_b_2_5_1_3.value() as u8,
            self.drawbar_b_3_8.value() as u8,
            self.drawbar_b_4_4.value() as u8,
            self.drawbar_b_5_2_2_3.value() as u8,
            self.drawbar_b_6_2.value() as u8,
            self.drawbar_b_7_1_3_5.value() as u8,
            self.drawbar_b_8_1_1_3.value() as u8,
            self.drawbar_b_9_1.value() as u8,
        ];

        engine.organ_a.percussion_enabled = self.organ_perc_enable.value();
        engine.organ_a.percussion_soft = self.organ_perc_soft.value();
        engine.organ_a.percussion_fast_decay = self.organ_perc_fast.value();
        engine.organ_a.percussion_third_harmonic = self.organ_perc_third.value();

        let spd = if self.rotary_speed_fast.value() {
            RotarySpeed::Fast
        } else {
            RotarySpeed::Slow
        };
        engine.organ_a.rotary_speed = spd;
        engine.organ_b.rotary_speed = spd;
        engine.organ_a.rotary_drive = self.rotary_drive.value();
        engine.organ_b.rotary_drive = self.rotary_drive.value();
    }

    /// Piano category, timbre, Touch (D169), string resonance and pedal noise.
    pub(crate) fn sync_piano(&self, engine: &mut Stage4Engine) {
        let cat = match self.piano_category.value() {
            1 => PianoCategory::Upright,
            2 => PianoCategory::Electric,
            3 => PianoCategory::Clav,
            4 => PianoCategory::Digital,
            5 => PianoCategory::Misc,
            _ => PianoCategory::Grand,
        };
        let timbre = match self.piano_timbre.value() {
            0 => PianoTimbre::Soft,
            2 => PianoTimbre::Bright,
            3 => PianoTimbre::Dyno1,
            4 => PianoTimbre::Dyno2,
            _ => PianoTimbre::Mid,
        };
        // The one global parameter replaces both layers' Touch only once the host moved it from
        // its value at the program load (D169); until then the per-layer program Touch plays.
        let touch = match self.piano_touch.value() {
            1 => VelocityTouch::Light,
            2 => VelocityTouch::Heavy,
            _ => VelocityTouch::Medium,
        };

        engine.piano_a.category = cat;
        engine.piano_b.category = cat;
        engine.piano_a.timbre = timbre;
        engine.piano_b.timbre = timbre;
        if self.program_overrides.touch_overrides() {
            engine.piano_a.touch = touch;
            engine.piano_b.touch = touch;
        }
        engine.piano_a.string_resonance_enabled = self.piano_string_res.value();
        engine.piano_b.string_resonance_enabled = self.piano_string_res.value();
        engine.piano_a.pedal_noise_enabled = self.piano_pedal_noise.value();
        engine.piano_b.pedal_noise_enabled = self.piano_pedal_noise.value();
    }

    /// The shared synth oscillator mode, filter and envelope (all three layers).
    pub(crate) fn sync_synth(&self, engine: &mut Stage4Engine) {
        let osc_m = match self.synth_osc_mode.value() {
            1 => SynthOscMode::Wavetable,
            2 => SynthOscMode::Fm,
            3 => SynthOscMode::Sample,
            _ => SynthOscMode::Analog,
        };
        let cutoff = self.synth_filter_cutoff.value();
        let resonance = self.synth_filter_resonance.value();
        let drive = self.synth_filter_drive.value();
        let atk = self.synth_attack.value();
        let dec = self.synth_decay.value();
        let rel = self.synth_release.value();

        for s in [
            &mut engine.synth_a,
            &mut engine.synth_b,
            &mut engine.synth_c,
        ] {
            s.osc_mode = osc_m;
            s.cutoff_hz = cutoff;
            s.resonance = resonance;
            s.filter_drive = drive;
            s.attack_sec = atk;
            s.decay_sec = dec;
            s.release_sec = rel;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nih_plug::prelude::BoolParam;

    #[test]
    fn each_group_writes_its_engine_fields_from_the_host_values() {
        let p = Stage4Params::default();
        let mut e = Stage4Engine::new(48_000.0);
        e.synth_a.cutoff_hz = 1.0;
        e.organ_a.drawbars = [0; 9];
        e.layer_levels[0].base = 0.0;
        e.piano_a.timbre = PianoTimbre::Soft;
        e.split_router.splits[0].midi_note = 0;
        p.sync_synth(&mut e);
        p.sync_organ(&mut e);
        p.sync_levels(&mut e);
        p.sync_piano(&mut e);
        p.sync_layers_and_splits(&mut e);
        assert_eq!(e.synth_a.cutoff_hz, 2500.0);
        assert_eq!(e.organ_a.drawbars[0], 8);
        assert!((e.layer_levels[0].base - 0.64).abs() < 1e-5);
        assert_eq!(e.piano_a.timbre, PianoTimbre::Mid);
        assert_eq!(e.split_router.splits[0].midi_note, 53);
    }

    #[test]
    fn the_layer_enables_follow_the_active_scene() {
        let p = Stage4Params {
            layer_scene_ii: BoolParam::new("s", true),
            ..Stage4Params::default()
        };
        let mut e = Stage4Engine::new(48_000.0);
        e.scene.scene_1_mask = 0xFF;
        p.sync_layers_and_splits(&mut e);
        assert!(e.scene.scene_2_active);
        assert_eq!(e.scene.scene_1_mask, 0xFF, "scene I is left alone");
    }
}
