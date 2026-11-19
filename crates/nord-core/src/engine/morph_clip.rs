//! Morph copy / paste / clear (manual p.43-44): one Morph source (Wheel, A.T., Ctrl Ped) holds a
//! depth for every morph-assigned parameter: 7 layer levels, 3 synth cutoffs, the 5 extra synth
//! slots of each synth layer (`synth_morph.rs`) and 9 per FX chain.

use super::*;

const FX_SLOTS: usize = 9;
const SYNTH_SLOTS: usize = crate::synth_morph::SLOTS;

/// Depths of one morph source across the whole program.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MorphClip {
    pub source: MorphSource,
    levels: [f32; 7],
    cutoff: [f32; 3],
    synth: [[f32; SYNTH_SLOTS]; 3],
    fx: [[f32; FX_SLOTS]; 7],
}

fn delta(p: &MorphParameter, s: MorphSource) -> f32 {
    match s {
        MorphSource::ModWheel => p.delta_wheel,
        MorphSource::ControlPedal => p.delta_pedal,
        MorphSource::Aftertouch => p.delta_aftertouch,
    }
}

fn set_delta(p: &mut MorphParameter, s: MorphSource, v: f32) {
    match s {
        MorphSource::ModWheel => p.delta_wheel = v,
        MorphSource::ControlPedal => p.delta_pedal = v,
        MorphSource::Aftertouch => p.delta_aftertouch = v,
    }
}

fn fx_slots(m: &mut FxMorphSet) -> [&mut MorphParameter; FX_SLOTS] {
    [
        &mut m.mod1_rate,
        &mut m.mod1_amount,
        &mut m.mod2_rate,
        &mut m.mod2_amount,
        &mut m.eq_mid_freq,
        &mut m.amp_drive,
        &mut m.delay_mix,
        &mut m.delay_feedback,
        &mut m.reverb_amount,
    ]
}

impl Stage4Engine {
    /// Reads the depths of morph `source` (control thread; no allocation).
    pub fn capture_morph(&self, source: MorphSource) -> MorphClip {
        MorphClip {
            source,
            levels: std::array::from_fn(|i| delta(&self.layer_levels[i], source)),
            cutoff: std::array::from_fn(|i| delta(&self.synth_cutoff_morph[i], source)),
            synth: std::array::from_fn(|i| {
                std::array::from_fn(|k| delta(self.synth_morph[i].slot(k), source))
            }),
            fx: std::array::from_fn(|i| {
                let mut m = self.fx_chains[i].morph;
                let slots = fx_slots(&mut m);
                std::array::from_fn(|k| delta(slots[k], source))
            }),
        }
    }

    /// Writes `clip` into morph `target` (a clip copied from Wheel may be pasted into A.T.).
    /// The previous state is kept for Undo.
    pub fn apply_morph_clip(&mut self, clip: &MorphClip, target: MorphSource) {
        self.begin_undoable(UndoOp::CopyPaste);
        self.write_morph(clip, target);
    }

    fn write_morph(&mut self, clip: &MorphClip, target: MorphSource) {
        for (p, v) in self.layer_levels.iter_mut().zip(clip.levels) {
            set_delta(p, target, v);
        }
        for (p, v) in self.synth_cutoff_morph.iter_mut().zip(clip.cutoff) {
            set_delta(p, target, v);
        }
        for (set, vals) in self.synth_morph.iter_mut().zip(clip.synth) {
            for (k, v) in vals.into_iter().enumerate() {
                set_delta(set.slot_mut(k), target, v);
            }
        }
        for (chain, vals) in self.fx_chains.iter_mut().zip(clip.fx) {
            for (p, v) in fx_slots(&mut chain.morph).into_iter().zip(vals) {
                set_delta(p, target, v);
            }
        }
    }

    /// Clears every depth of morph `source` (undoable, manual p.43).
    pub fn clear_morph(&mut self, source: MorphSource) {
        self.begin_undoable(UndoOp::ClearMorph);
        let empty = MorphClip {
            source,
            levels: [0.0; 7],
            cutoff: [0.0; 3],
            synth: [[0.0; SYNTH_SLOTS]; 3],
            fx: [[0.0; FX_SLOTS]; 7],
        };
        self.write_morph(&empty, source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_and_paste_to_another_source() {
        let mut e = Stage4Engine::new(48_000.0);
        e.layer_levels[3].delta_wheel = 0.25;
        e.synth_cutoff_morph[2].delta_wheel = -0.5;
        e.fx_chains[6].morph.reverb_amount.delta_wheel = 0.6;
        let clip = e.capture_morph(MorphSource::ModWheel);
        e.apply_morph_clip(&clip, MorphSource::Aftertouch);
        assert_eq!(e.layer_levels[3].delta_aftertouch, 0.25);
        assert_eq!(e.synth_cutoff_morph[2].delta_aftertouch, -0.5);
        assert_eq!(e.fx_chains[6].morph.reverb_amount.delta_aftertouch, 0.6);
        assert_eq!(e.layer_levels[3].delta_wheel, 0.25);
        assert_eq!(e.layer_levels[3].delta_pedal, 0.0);
    }

    #[test]
    fn clear_morph_zeroes_one_source_and_undoes() {
        let mut e = Stage4Engine::new(48_000.0);
        e.layer_levels[0].delta_wheel = 0.4;
        e.layer_levels[0].delta_pedal = 0.3;
        e.fx_chains[1].morph.delay_mix.delta_wheel = 0.2;
        e.clear_morph(MorphSource::ModWheel);
        assert_eq!(e.layer_levels[0].delta_wheel, 0.0);
        assert_eq!(e.fx_chains[1].morph.delay_mix.delta_wheel, 0.0);
        assert_eq!(e.layer_levels[0].delta_pedal, 0.3);
        assert_eq!(e.undo(), Some(UndoOp::ClearMorph));
        assert_eq!(e.layer_levels[0].delta_wheel, 0.4);
    }

    #[test]
    fn the_five_extra_synth_slots_copy_paste_and_clear_with_the_rest() {
        let mut e = Stage4Engine::new(48_000.0);
        e.synth_morph[1].env_amount.delta_wheel = 0.3;
        e.synth_morph[2].resonance.delta_wheel = -0.2;
        e.synth_morph[0].lfo_rate.delta_pedal = 0.5;
        let clip = e.capture_morph(MorphSource::ModWheel);
        e.apply_morph_clip(&clip, MorphSource::Aftertouch);
        assert_eq!(e.synth_morph[1].env_amount.delta_aftertouch, 0.3);
        assert_eq!(e.synth_morph[2].resonance.delta_aftertouch, -0.2);
        assert_eq!(e.synth_morph[0].lfo_rate.delta_aftertouch, 0.0);
        e.clear_morph(MorphSource::ModWheel);
        assert!(!e.synth_morph[1].is_active() || e.synth_morph[1].env_amount.delta_wheel == 0.0);
        assert_eq!(e.synth_morph[1].env_amount.delta_wheel, 0.0);
        assert_eq!(e.synth_morph[2].resonance.delta_wheel, 0.0);
        assert_eq!(
            e.synth_morph[0].lfo_rate.delta_pedal, 0.5,
            "other sources stay"
        );
        assert_eq!(e.undo(), Some(UndoOp::ClearMorph));
        assert_eq!(e.synth_morph[1].env_amount.delta_wheel, 0.3);
    }

    #[test]
    fn after_clear_morph_the_wheel_no_longer_moves_the_five_slots() {
        let mut e = Stage4Engine::new(48_000.0);
        e.firmware_morph = true;
        e.synth_a.filter_env_amount = 0.25;
        e.synth_morph[0].env_amount.base = 0.25;
        e.synth_morph[0].env_amount.delta_wheel = 0.5;
        e.morph_controllers.mod_wheel = 1.0;
        e.render_frame();
        assert!(
            (e.synth_a.filter_env_amount - 0.75).abs() < 0.01,
            "wheel moves it"
        );
        e.clear_morph(MorphSource::ModWheel);
        e.render_frame();
        assert!(
            (e.synth_a.filter_env_amount - 0.25).abs() < 1e-4,
            "base is back"
        );
        e.morph_controllers.mod_wheel = 0.5;
        e.render_frame();
        assert!(
            (e.synth_a.filter_env_amount - 0.25).abs() < 1e-4,
            "wheel is silent"
        );
        // a live edit after the clear sticks
        e.synth_a.filter_env_amount = 0.6;
        e.render_frame();
        assert!((e.synth_a.filter_env_amount - 0.6).abs() < 1e-4);
        // paste brings the depth back on top of that edit
        e.synth_morph[1].env_amount.delta_wheel = 0.0;
        let mut clip_src = Stage4Engine::new(48_000.0);
        clip_src.synth_morph[0].env_amount.delta_wheel = 0.2;
        let clip = clip_src.capture_morph(MorphSource::ModWheel);
        e.apply_morph_clip(&clip, MorphSource::ModWheel);
        e.morph_controllers.mod_wheel = 1.0;
        e.render_frame();
        assert!((e.synth_a.filter_env_amount - 0.8).abs() < 0.01);
    }
}
