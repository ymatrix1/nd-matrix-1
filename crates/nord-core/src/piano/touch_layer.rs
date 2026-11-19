//! Program Touch of a [`PianoLayer`]: which velocity curve a note-on plays.
//!
//! `PianoLayer::firmware_touch` (default `true` since D165) selects the firmware curves of
//! [`super::touch_curve`] for the layer's `touch` setting; off (`--legacy-piano-touch`), every
//! `.npno` note plays the Medium curve (the factory default, VERIFIED) and the other sounds the
//! ASSUMED power curves of `VelocityTouch::map_velocity`.

use super::touch_curve::TouchCurve;
use super::PianoLayer;

impl PianoLayer {
    /// The Touch curve the firmware level law plays: the layer's setting when
    /// `firmware_touch` is on, Medium otherwise.
    pub fn touch_curve(&self) -> TouchCurve {
        if self.firmware_touch {
            TouchCurve::from(self.touch)
        } else {
            TouchCurve::Medium
        }
    }

    /// Voice amplitude of sounds without a firmware level model. Off: the ASSUMED power curve of
    /// the layer's Touch. On: the Medium response times the verified level ratio of the curve
    /// (`TouchCurve::gain_vs_medium`), so Medium keeps its response and only the Touch difference
    /// is firmware derived.
    pub(super) fn touch_fallback_amp(&self, velocity: u8) -> f32 {
        if self.firmware_touch {
            super::VelocityTouch::Medium.map_velocity(velocity)
                * self.touch_curve().gain_vs_medium(velocity)
        } else {
            self.touch.map_velocity(velocity)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::npno_level::{test_image, NpnoLevel};
    use super::super::{NpnoSound, VelocityTouch};
    use super::*;
    use std::sync::Arc;

    fn layer_with_model() -> PianoLayer {
        let image = test_image(60, &[(0, 66, 0), (0, 56, 16), (0, 36, 46)], 40);
        let model = Arc::new(NpnoLevel::parse(&image).unwrap());
        let mut sound = NpnoSound::new_synthetic("fw", 60, &[0.1; 64], 48_000.0);
        let stroke = sound.strokes[0].clone();
        sound.strokes = vec![stroke; 3];
        sound.firmware_level = Some(model);
        let mut layer = PianoLayer::new(48_000.0);
        layer.set_sound(Some(Arc::new(sound)));
        layer
    }

    #[test]
    fn default_follows_the_setting_and_legacy_is_medium() {
        let mut layer = PianoLayer::new(48_000.0);
        assert!(layer.firmware_touch, "on by default since D165");
        layer.touch = VelocityTouch::Light;
        assert_eq!(layer.touch_curve(), TouchCurve::Light);
        layer.touch = VelocityTouch::Heavy;
        assert_eq!(layer.touch_curve(), TouchCurve::Heavy);
        layer.firmware_touch = false;
        assert_eq!(layer.touch_curve(), TouchCurve::Medium);
    }

    #[test]
    fn fallback_is_the_assumed_power_curve_off_and_medium_scaled_on() {
        let mut layer = PianoLayer::new(48_000.0);
        layer.touch = VelocityTouch::Heavy;
        layer.firmware_touch = false;
        assert_eq!(
            layer.touch_fallback_amp(64),
            VelocityTouch::Heavy.map_velocity(64)
        );
        layer.firmware_touch = true;
        let medium = VelocityTouch::Medium.map_velocity(64);
        let heavy = layer.touch_fallback_amp(64);
        assert!((heavy - medium * TouchCurve::Heavy.gain_vs_medium(64)).abs() < 1e-7);
        layer.touch = VelocityTouch::Medium;
        assert_eq!(layer.touch_fallback_amp(64), medium, "Medium is unchanged");
    }

    #[test]
    fn npno_note_on_follows_the_touch_curve_only_when_enabled() {
        let level = |touch: VelocityTouch, on: bool| {
            let mut layer = layer_with_model();
            layer.touch = touch;
            layer.firmware_touch = on;
            layer.note_on(60, 40);
            layer.voices[0].amp
        };
        let medium = level(VelocityTouch::Medium, true);
        assert_eq!(
            level(VelocityTouch::Heavy, false),
            medium,
            "off ignores touch"
        );
        assert!(level(VelocityTouch::Heavy, true) < medium);
        assert!(level(VelocityTouch::Light, true) > medium);
    }
}
