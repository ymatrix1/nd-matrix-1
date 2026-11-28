//! Notes struck with the damper pedal down: the firmware voice list is the same.
//!
//! Evidence: `docs/research/piano-extra-voices.md` s2 (`tools/arm_piano_pedal.py`): on the real
//! firmware a note-on after CC64 = 127 runs the same level method calls (same stroke records,
//! same Q23 gains) as without the pedal; the pedal only changes when the voices are released
//! (the note-off is held, the pedal-up releases). So `start_firmware_voices` can serve
//! pedal-down notes; the engine's sustain logic (`note_off`, `set_damper_pedal`) already holds
//! and releases the voices it starts. On by default since D261 (issue #679: the legacy
//! damper-stroke path started one voice where the firmware starts two, so a note struck with the
//! pedal down was about 6 dB quieter); `PianoLayer::firmware_pedal_voices = false` restores it.

use super::PianoLayer;

impl PianoLayer {
    /// `true` when a note-on may use the firmware voice list now: always with the pedal up, with
    /// the pedal down unless `firmware_pedal_voices` is cleared (then the legacy damper-stroke
    /// path plays the note).
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
        // 0.5 s sine at the root pitch, so a 100 ms level window never runs off the sample.
        let sine: Vec<f32> = (0..24_000)
            .map(|i| 0.5 * (std::f32::consts::TAU * 261.625_56 * i as f32 / 48_000.0).sin())
            .collect();
        let mut snd = NpnoSound::new_synthetic("fw", 60, &sine, 48_000.0);
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
    fn pedal_down_voices_are_the_default_and_the_flag_restores_the_legacy_path() {
        assert!(
            PianoLayer::new(48_000.0).firmware_pedal_voices,
            "default since D261"
        );
        let mut layer = layer(false);
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

    /// RMS and peak of the left channel over the first `ms` after a note-on.
    fn level(layer: &mut PianoLayer, ms: usize) -> (f32, f32) {
        let n = 48 * ms;
        let (mut sq, mut peak) = (0.0f64, 0.0f32);
        for _ in 0..n {
            let x = layer.render_frame()[0];
            sq += f64::from(x) * f64::from(x);
            peak = peak.max(x.abs());
        }
        (((sq / n as f64).sqrt()) as f32, peak)
    }

    /// Lets the pedal-press thump and brush noise (`pedal_noise_enabled`) die away, so the
    /// window that follows measures the note and not the pedal.
    fn settle(layer: &mut PianoLayer) {
        for _ in 0..48 * 800 {
            layer.render_frame();
        }
    }

    /// Issue #679: a note struck with the pedal already down was quieter than the same note
    /// struck before the pedal. Fixture: sine stroke, 100 ms window, pedal noise settled.
    /// Measured pedal down over pedal up before the fix (legacy path, one voice against two):
    /// RMS and peak -6.02 dB at velocities 20 and 64, -6.54 dB at 110; after: 0.00 dB at all three.
    #[test]
    fn pedal_down_note_is_as_loud_as_pedal_up_note() {
        for vel in [20u8, 64, 110] {
            let mut up = layer(true);
            settle(&mut up);
            up.note_on(60, vel);
            let (rms_up, peak_up) = level(&mut up, 100);
            let mut down = layer(true);
            down.set_damper_pedal(true, 64);
            settle(&mut down);
            down.note_on(60, vel);
            let (rms_down, peak_down) = level(&mut down, 100);
            assert!(rms_up > 1e-3, "fixture must sound");
            assert!(
                (rms_down / rms_up - 1.0).abs() < 0.01 && (peak_down / peak_up - 1.0).abs() < 0.01,
                "vel {vel}: rms {rms_down} vs {rms_up}, peak {peak_down} vs {peak_up}"
            );
        }
    }

    /// The legacy path (opt-out) keeps its one voice: guards the measurement above.
    #[test]
    fn legacy_pedal_down_note_is_quieter_at_high_velocity() {
        let mut up = layer(false);
        settle(&mut up);
        up.note_on(60, 110);
        let (rms_up, _) = level(&mut up, 100);
        let mut down = layer(false);
        down.set_damper_pedal(true, 64);
        settle(&mut down);
        down.note_on(60, 110);
        let (rms_down, _) = level(&mut down, 100);
        assert!(rms_down < 0.6 * rms_up, "{rms_down} vs {rms_up}");
    }
}
