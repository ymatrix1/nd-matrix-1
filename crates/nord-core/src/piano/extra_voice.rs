//! Note-on of the firmware voices: the extra first voice and the main voice of an `.npno` sound.
//!
//! Evidence: `docs/research/piano-sound-load.md` sections 9 and 10. For sounds with a parsed
//! level model ([`super::npno_level::NpnoLevel`]) a note-on starts the voices the firmware starts
//! (extra first voice on classes 1, 2, 3, 5, 6, then the main voice), each on its own stroke record
//! with the gain `level_q23 / 2^23` (VERIFIED: the float the ARM writes to the SMP voice record).
//! The voices are ordinary [`PianoVoice`](super::PianoVoice) slots, pre-allocated in the layer;
//! nothing here allocates. Release treats them as one note (`extra` voices do not trigger a
//! second release sample).

use super::slew_voice::{rate_from_record, SlewVoice};
use super::{PianoLayer, PianoVoice};

/// Amplitude cap of one voice (the engine's `0..=1` voice amplitude).
const AMP_MAX: f32 = 1.0;
const Q23_ONE: f32 = 8_388_608.0;

impl PianoLayer {
    /// Starts the firmware voices of `note`. Returns `false` (nothing started) when the layer has
    /// no sound with a level model, the note is unmapped, or a record has no stroke; the caller
    /// then falls back to the legacy single-voice path.
    pub(super) fn start_firmware_voices(
        &mut self,
        note: u8,
        velocity: u8,
        effective_note: u8,
        freq: f32,
        detune_ratio: f64,
        gain: f32,
    ) -> bool {
        // Pedal down: the legacy path selects the damper-down strokes unless the firmware voice
        // list is opted in (`pedal_voices.rs`).
        if !self.firmware_voices_allowed() {
            return false;
        }
        let Some(sound) = self.sound.clone() else {
            return false;
        };
        let Some(model) = sound.firmware_level.as_ref() else {
            return false;
        };
        let voices = model.voices_touch(effective_note, velocity, self.touch_curve());
        // Every voice needs a stroke; otherwise keep the legacy path.
        let usable = !voices.is_empty()
            && voices
                .iter()
                .all(|v| sound.strokes.get(usize::from(v.record)).is_some());
        if !usable {
            return false;
        }
        for v in &mut self.voices {
            if v.active && !v.is_release_sample && v.note == note && !v.releasing {
                v.releasing = true;
                v.soft_release = false;
                v.sustained = false;
                v.sostenuto_held = false;
            }
        }
        let una_corda_scale = if self.soft_pedal_down { 0.76 } else { 1.0 };
        let cents = f32::from(sound.tuning_cents[usize::from(effective_note.min(127))]);
        let last = voices.len() - 1;
        for (position, voice) in voices.iter().enumerate() {
            // The SMP mutes every voice but the last (ramp `F-2 = F3 = 0`): not started.
            if self.firmware_slew && position != last {
                continue;
            }
            let Some(stroke) = sound.strokes.get(usize::from(voice.record)) else {
                continue;
            };
            let semitones =
                (f32::from(effective_note) - f32::from(stroke.root_key)) + cents / 100.0;
            let step = f64::from(2.0f32.powf(semitones / 12.0))
                * (f64::from(stroke.sample_rate) / f64::from(self.sample_rate));
            let level = (voice.amplitude_q23 as f32 / Q23_ONE).min(AMP_MAX);
            let compressed = match self.dyn_comp {
                1 => level.powf(0.82),
                2 => level.powf(0.68),
                3 => level.powf(0.55),
                _ => level,
            };
            let slew = if self.firmware_slew && level > 0.0 {
                SlewVoice::start(
                    level,
                    rate_from_record(voice.decay, semitones),
                    self.sample_rate,
                )
            } else {
                SlewVoice::default()
            };
            // The slew already carries the level; keep only the dynamics and soft pedal ratio.
            let amp = if slew.is_active() {
                compressed / level * una_corda_scale
            } else {
                compressed * una_corda_scale
            };
            let slot = self.allocate_voice_slot(None);
            self.voices[slot] = PianoVoice {
                active: true,
                note,
                last_velocity: velocity,
                freq,
                slew,
                amp: amp.clamp(0.0, AMP_MAX),
                env: 1.0,
                gain: gain.clamp(0.0, 1.0),
                stroke_idx: Some(usize::from(voice.record)),
                sample_step: step,
                sample_step_detune: step * detune_ratio,
                extra: position != last,
                ..PianoVoice::default()
            };
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::npno_level::{test_image, NpnoLevel};
    use super::super::{NpnoSound, PianoLayer, PianoVoice};
    use std::sync::Arc;

    /// Synthetic sound with `strokes` identical one-key strokes (record `k` = stroke `k`).
    fn sound_with(image: &[u8], strokes: usize) -> Arc<NpnoSound> {
        let mut snd = NpnoSound::new_synthetic("fw", 60, &[0.2; 256], 48_000.0);
        let stroke = snd.strokes[0].clone();
        snd.strokes = vec![stroke; strokes];
        snd.firmware_level = NpnoLevel::parse(image).map(Arc::new);
        Arc::new(snd)
    }

    fn layer_with(image: &[u8], strokes: usize, on: bool) -> PianoLayer {
        let mut layer = PianoLayer::new(48_000.0);
        layer.set_sound(Some(sound_with(image, strokes)));
        layer.firmware_velocity_level = on;
        layer
    }

    fn two_voice_image() -> Vec<u8> {
        // Class 5, root 60: layer 0 strokes (offset, threshold); extra voice = layer 0 at T + 20.
        let mut image = test_image(60, &[(0, 66, 0), (0, 56, 16), (0, 36, 46)], 40);
        image[0x44] = 5;
        image
    }

    #[test]
    fn two_voice_class_starts_two_voices_with_the_model_gains() {
        let image = two_voice_image();
        let model = NpnoLevel::parse(&image).unwrap();
        let mut layer = layer_with(&image, 3, true);
        layer.note_on(60, 64);
        let want = model.voices(60, 64);
        assert_eq!(want.len(), 2);
        let active: Vec<_> = layer.voices.iter().filter(|v| v.active).collect();
        assert_eq!(active.len(), 2);
        for (v, w) in active.iter().zip(want.iter()) {
            assert!((v.amp - w.amplitude_q23 as f32 / 8_388_608.0).abs() < 1e-6);
            assert_eq!(v.stroke_idx, Some(usize::from(w.record)));
        }
        assert!(active[0].extra && !active[1].extra);
    }

    #[test]
    fn single_voice_class_starts_one_voice() {
        let mut image = two_voice_image();
        image[0x44] = 4;
        let mut layer = layer_with(&image, 3, true);
        layer.note_on(60, 64);
        assert_eq!(layer.voices.iter().filter(|v| v.active).count(), 1);
    }

    #[test]
    fn both_voices_release_together_with_one_release_sample_path() {
        let mut layer = layer_with(&two_voice_image(), 3, true);
        layer.note_on(60, 64);
        layer.note_off(60);
        assert!(layer
            .voices
            .iter()
            .filter(|v| v.active)
            .all(|v| v.releasing));
    }

    #[test]
    fn retriggering_a_note_damps_both_old_voices() {
        let mut layer = layer_with(&two_voice_image(), 3, true);
        layer.note_on(60, 64);
        layer.note_on(60, 90);
        let (old, new): (Vec<&PianoVoice>, Vec<&PianoVoice>) = layer
            .voices
            .iter()
            .filter(|v| v.active)
            .partition(|v| v.releasing);
        assert_eq!((old.len(), new.len()), (2, 2));
    }

    #[test]
    fn missing_stroke_or_unmapped_note_falls_back_to_legacy() {
        let image = two_voice_image();
        // Only one stroke for three records: record 1/2 have no stroke -> legacy path.
        let mut layer = layer_with(&image, 1, true);
        layer.note_on(60, 64);
        let started = layer.voices.iter().filter(|v| v.active).count();
        assert_eq!(started, 1, "legacy path starts exactly one voice");
        assert!(!layer.voices.iter().any(|v| v.extra));
    }

    #[test]
    fn dyn_comp_and_una_corda_scale_each_voice() {
        let image = two_voice_image();
        let mut plain = layer_with(&image, 3, true);
        plain.note_on(60, 64);
        let mut soft = layer_with(&image, 3, true);
        soft.soft_pedal_down = true;
        soft.note_on(60, 64);
        let a: Vec<f32> = plain
            .voices
            .iter()
            .filter(|v| v.active)
            .map(|v| v.amp)
            .collect();
        let b: Vec<f32> = soft
            .voices
            .iter()
            .filter(|v| v.active)
            .map(|v| v.amp)
            .collect();
        for (x, y) in a.iter().zip(&b) {
            assert!((y - x * 0.76).abs() < 1e-6);
        }
    }

    #[test]
    fn rendering_the_two_voices_produces_finite_audio() {
        let mut layer = layer_with(&two_voice_image(), 3, true);
        layer.note_on(60, 100);
        for _ in 0..512 {
            let [l, r] = layer.render_frame();
            assert!(l.is_finite() && r.is_finite());
        }
    }

    /// Record `k` of the file is stroke `k` of the loaded sound: the firmware record index of a
    /// voice selects the right sample. Checked on real files when `NS4_NPNO_DIR` points at a
    /// directory with `.npno` files (skipped otherwise; the files are never committed).
    #[test]
    fn record_index_equals_stroke_index_on_real_files() {
        let Ok(dir) = std::env::var("NS4_NPNO_DIR") else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let sound = NpnoSound::from_npno_bytes(&bytes).unwrap();
            let model = sound.firmware_level.as_ref().expect("level model parses");
            for (k, stroke) in sound.strokes.iter().enumerate() {
                assert_eq!(
                    model.record_root(k as u16),
                    Some(stroke.root_key),
                    "{:?} record {k}",
                    entry.path()
                );
            }
        }
    }

    #[test]
    fn slew_starts_only_the_main_voice_with_the_firmware_gain_and_releases() {
        let image = two_voice_image();
        let model = NpnoLevel::parse(&image).unwrap();
        let level = model.voices(60, 64).last().unwrap().amplitude_q23 as f32 / 8_388_608.0;
        let mut layer = layer_with(&image, 3, true);
        layer.firmware_slew = true;
        layer.note_on(60, 64);
        let active: Vec<_> = layer.voices.iter().filter(|v| v.active).collect();
        assert_eq!(active.len(), 1, "the muted extra voice is not started");
        assert!(active[0].slew.is_active() && !active[0].extra);
        assert!(
            (active[0].amp - 1.0).abs() < 1e-6,
            "level lives in the slew"
        );
        let (gl, gr) = layer
            .voices
            .iter()
            .find(|v| v.active)
            .unwrap()
            .slew
            .clone()
            .tick();
        assert!(
            (gl - 0.21875 * level).abs() < 2e-3 * level && gl == gr,
            "{gl} vs {level}"
        );
        for _ in 0..4800 {
            layer.render_frame();
        }
        layer.note_off(60);
        for _ in 0..48_000 * 3 {
            layer.render_frame();
        }
        assert!(layer.voices.iter().all(|v| !v.active), "release fades out");
    }

    /// With the pedal-down voices opted out (`firmware_pedal_voices = false`, the pre-D262
    /// default) the firmware voice list (and with it the slew) is bypassed and the legacy
    /// damper-stroke path plays the note: `chain + slew` clip replays mixed both paths
    /// (25 to 77 % of the Part A piano notes start with the pedal down,
    /// `docs/research/piano-decay-attack.md` s6).
    #[test]
    fn pedal_down_bypasses_the_slew_voice() {
        let image = two_voice_image();
        let mut layer = layer_with(&image, 3, true);
        layer.firmware_slew = true;
        layer.firmware_pedal_voices = false;
        layer.set_damper_pedal(true, 64);
        layer.note_on(60, 64);
        let active: Vec<_> = layer.voices.iter().filter(|v| v.active).collect();
        assert!(!active.is_empty());
        assert!(active.iter().all(|v| !v.slew.is_active()));
        layer.set_damper_pedal(false, 64);
        layer.note_on(60, 64);
        assert!(layer
            .voices
            .iter()
            .any(|v| v.active && !v.releasing && v.slew.is_active()));
    }

    /// Layer whose sound also has a key-up (release) zone on record 2.
    fn layer_with_release(slew: bool) -> PianoLayer {
        use super::super::PianoSampleZone;
        let mut snd = (*sound_with(&two_voice_image(), 3)).clone();
        snd.zones = vec![PianoSampleZone {
            key_low: 0,
            key_high: 127,
            vel_low: 1,
            vel_high: 127,
            root_key: 60,
            tune_cents: 0,
            trigger_release: true,
            damper_down: false,
            stroke_idx: 2,
        }];
        let mut layer = PianoLayer::new(48_000.0);
        layer.set_sound(Some(Arc::new(snd)));
        layer.firmware_velocity_level = true;
        layer.firmware_slew = slew;
        layer
    }

    #[test]
    fn slew_note_off_starts_a_firmware_release_sample_not_the_engine_constant() {
        let mut layer = layer_with_release(true);
        layer.note_on(60, 64);
        layer.note_off(60);
        let rel: Vec<_> = layer
            .voices
            .iter()
            .filter(|v| v.active && v.is_release_sample)
            .collect();
        assert_eq!(rel.len(), 1);
        assert!(rel[0].slew.is_active() && (rel[0].amp - 1.0).abs() < 1e-6);
        // Without the slew the engine constant 0.30 of the voice amplitude applies.
        let mut plain = layer_with_release(false);
        plain.note_on(60, 64);
        plain.note_off(60);
        let r = plain
            .voices
            .iter()
            .find(|v| v.active && v.is_release_sample)
            .unwrap();
        assert!(!r.slew.is_active() && r.amp < 0.31);
    }

    #[test]
    fn slew_off_keeps_two_voices_and_no_slew() {
        let image = two_voice_image();
        let mut layer = layer_with(&image, 3, true);
        layer.note_on(60, 64);
        let active: Vec<_> = layer.voices.iter().filter(|v| v.active).collect();
        assert_eq!(active.len(), 2);
        assert!(active.iter().all(|v| !v.slew.is_active()));
    }
}
