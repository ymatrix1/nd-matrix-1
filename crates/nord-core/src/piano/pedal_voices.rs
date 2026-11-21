//! Notes struck with the damper pedal down: the firmware voice list is the same.
//!
//! Evidence: `docs/research/piano-extra-voices.md` s2 (`tools/arm_piano_pedal.py`): on the real
//! firmware a note-on after CC64 = 127 runs the same level method calls (same stroke records,
//! same Q23 gains) as without the pedal; the pedal only changes when the voices are released
//! (the note-off is held, the pedal-up releases). So `start_firmware_voices` can serve
//! pedal-down notes; the engine's sustain logic (`note_off`, `set_damper_pedal`) already holds
//! and releases the voices it starts. Opt-in via `PianoLayer::firmware_pedal_voices` (D22).

use super::PianoLayer;

impl PianoLayer {
    /// `true` when a note-on may use the firmware voice list now: always with the pedal up, with
    /// the pedal down only when `firmware_pedal_voices` is set (otherwise the legacy
    /// damper-stroke path plays the note).
    pub(super) fn firmware_voices_allowed(&self) -> bool {
        !self.damper_pedal_down || self.firmware_pedal_voices
    }

    /// `(stroke index, extra voice, amplitude)` of every active non-release voice, for the census
    /// tool (`piano_voice_census`). Allocates: never call from the audio path.
    pub fn active_voice_strokes(&self) -> Vec<(Option<usize>, bool, f32)> {
        self.voices
            .iter()
            .filter(|v| v.active && !v.is_release_sample)
            .map(|v| (v.stroke_idx, v.extra, v.amp))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::npno_level::{test_image, NpnoLevel};
    use super::super::{NpnoSound, PianoLayer};
    use std::sync::Arc;

    fn layer(pedal_voices: bool) -> PianoLayer {
        // Class 5, root 60: three layer-0 strokes; extra voice = layer 0 at T + 20.
        let mut image = test_image(60, &[(0, 66, 0), (0, 56, 16), (0, 36, 46)], 40);
        image[0x44] = 5;
        let mut snd = NpnoSound::new_synthetic("fw", 60, &[0.2; 256], 48_000.0);
        let stroke = snd.strokes[0].clone();
        snd.strokes = vec![stroke; 3];
        snd.firmware_level = NpnoLevel::parse(&image).map(Arc::new);
        let mut layer = PianoLayer::new(48_000.0);
        layer.set_sound(Some(Arc::new(snd)));
        layer.firmware_velocity_level = true;
        layer.firmware_pedal_voices = pedal_voices;
        layer
    }

    fn amps(layer: &PianoLayer) -> Vec<(Option<usize>, f32)> {
        layer
            .voices
            .iter()
            .filter(|v| v.active)
            .map(|v| (v.stroke_idx, v.amp))
            .collect()
    }

    #[test]
    fn default_pedal_down_keeps_the_legacy_path() {
        let mut layer = layer(false);
        assert!(
            !layer.firmware_pedal_voices,
            "opt-in: the layer default is off"
        );
        layer.set_damper_pedal(true, 64);
        assert!(!layer.firmware_voices_allowed());
        layer.note_on(60, 64);
        assert!(!layer.voices.iter().any(|v| v.active && v.extra));
    }

    #[test]
    fn opt_in_pedal_down_starts_the_same_voices_as_pedal_up() {
        let mut up = layer(true);
        up.note_on(60, 64);
        let mut down = layer(true);
        down.set_damper_pedal(true, 64);
        assert!(down.firmware_voices_allowed());
        down.note_on(60, 64);
        assert_eq!(amps(&up).len(), 2);
        assert_eq!(amps(&up), amps(&down), "same records and gains");
        assert!(down.voices.iter().any(|v| v.active && v.extra));
    }

    #[test]
    fn opt_in_voices_are_held_by_the_pedal_and_released_by_its_lift() {
        let mut layer = layer(true);
        layer.set_damper_pedal(true, 64);
        layer.note_on(60, 64);
        layer.note_off(60);
        assert!(layer
            .voices
            .iter()
            .filter(|v| v.active)
            .all(|v| v.sustained && !v.releasing));
        layer.set_damper_pedal(false, 64);
        assert!(layer
            .voices
            .iter()
            .filter(|v| v.active && !v.is_release_sample)
            .all(|v| v.releasing));
    }

    #[test]
    fn active_voice_strokes_lists_the_started_voices() {
        let mut layer = layer(true);
        assert!(layer.active_voice_strokes().is_empty());
        layer.note_on(60, 64);
        let list = layer.active_voice_strokes();
        assert_eq!(list.len(), 2);
        assert!(list[0].1 && !list[1].1, "extra voice first, main last");
    }
}
