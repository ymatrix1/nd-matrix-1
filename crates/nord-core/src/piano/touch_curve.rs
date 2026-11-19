//! The program Touch velocity curves of the Piano section (`KB TOUCH`), from the firmware.
//!
//! The Piano section of a program has one velocity response setting, edit id `0x159`
//! (`Edt_Pno_kKbdTouch`, values 0..=2, default 1; `docs/research/arm-edit-params.csv`). The
//! handler `0xC011AA24` stores `curveId = u32[0xC02BBF98 + 0x678 + 4 * value]` = `10, 11, 13` into
//! field `+0x1C` of the piano layer object. The voice-init function `0xC00AA520` then reads the
//! velocity position of every note as `T = byte[0xC0532664 + (curveId << 10) + r8]`, where `r8` is
//! the internal velocity of the note (`s16` table `0xC02B3128`, curve 6 in force: `r8(1) = 17`,
//! `r8(64) = 203`, `r8(127) = 656`), see [`super::velocity_curve`]. `T` picks the stroke records
//! and gives the level attenuation `X`, so the Touch changes both the level and the layer of a
//! note.
//!
//! The tables below are the compositions `T(v)` for the three curve ids (127 bytes each, derived
//! numbers; the firmware tables are not copied). Curve 11 (value 1, the default) is
//! [`super::velocity_curve::T_BY_VELOCITY`].
//!
//! VERIFIED: all three tables equal the byte the ARM reads for a MIDI note-on through the real
//! firmware (UART2 MIDI parser, Clavinet D6 loaded, `[ctx + 0x1C]` set to 10, 11 and 13, velocities
//! 1, 2, 10, 33, 64, 90, 100, 120 and 127): 27 of 27 (`tools/arm_touch_curves.py arm-check`,
//! `docs/research/piano-touch-curves.md`). The table fingerprint is a unit test.
//!
//! VERIFIED: the program file bits equal the edit value: file bit `1991..1992` (layer A, layer B
//! `+96`) set to 0 / 1 / 2 reaches the Touch handler `0xC011AA2C` with the same index in 6 of 6
//! program recalls (`tools/arm_program_load.py`, D99, `docs/research/piano-touch-curves.md` s4);
//! the factory set reads 156 / 208 / 20 for 0 / 1 / 2 (`piano_touch_census`, no value 3).
//!
//! VERIFIED: the firmware recall of every factory program holds the Touch the decoder reads, both
//! layers: 384 of 384 programs, 768 of 768 layer values (332 / 414 / 22 for 0 / 1 / 2),
//! `tools/arm_program_touch_bits.py batch` (data `docs/research/data/piano-touch-recall.json`,
//! unit test `test_committed_recall_equals_the_decoder_census_for_every_factory_program`), D165.
//!
//! EMPIRICAL: the name of each edit value follows the monotone order of the curves (value 0 needs
//! the most force: `T` is largest) and the `ns4decode` map (`0 heavy, 1 med, 2 light`).
//!
//! ASSUMED (not covered by the run above): the response of sounds without a `.npno` level model
//! ([`TouchCurve::gain_vs_medium`] only scales the Medium response by the curve difference), the
//! System-menu MIDI velocity curves, and the hardware level of the Heavy and Light curves (the
//! reference clips are Medium).

use super::velocity_curve::{x_from_position, T_BY_VELOCITY};
use crate::piano::VelocityTouch;

/// Firmware edit value 0 (`curveId` 10): the most force for a given level.
const T_HEAVY: [u8; 127] = [
    200, 200, 200, 200, 200, 200, 200, 198, 195, 192, 190, 188, 185, 182, 181, 179, 176, 174, 172,
    169, 167, 165, 163, 160, 158, 157, 155, 152, 151, 149, 147, 144, 143, 140, 139, 137, 135, 133,
    131, 129, 127, 125, 124, 122, 120, 118, 116, 115, 113, 111, 109, 108, 106, 104, 103, 101, 99,
    98, 96, 94, 93, 91, 89, 88, 86, 85, 83, 81, 80, 78, 77, 75, 74, 72, 71, 69, 68, 66, 65, 63, 62,
    60, 59, 57, 56, 54, 53, 51, 50, 49, 47, 46, 44, 43, 42, 40, 39, 38, 36, 35, 33, 32, 31, 29, 28,
    27, 25, 24, 23, 21, 20, 19, 18, 16, 15, 14, 12, 11, 10, 8, 7, 6, 5, 3, 2, 1, 0,
];

/// Firmware edit value 2 (`curveId` 13): the least force for a given level.
const T_LIGHT: [u8; 127] = [
    200, 183, 180, 178, 175, 173, 170, 168, 166, 163, 162, 159, 156, 154, 153, 151, 148, 146, 144,
    142, 140, 138, 137, 134, 132, 131, 129, 127, 125, 124, 121, 119, 118, 116, 115, 113, 111, 109,
    107, 105, 104, 102, 100, 99, 97, 95, 94, 92, 91, 89, 87, 86, 84, 83, 81, 80, 78, 76, 75, 74,
    72, 70, 69, 68, 66, 65, 63, 62, 60, 59, 58, 56, 55, 53, 52, 50, 49, 48, 46, 45, 44, 42, 41, 39,
    38, 37, 36, 34, 33, 32, 30, 29, 28, 27, 25, 24, 23, 21, 20, 19, 18, 16, 15, 14, 13, 11, 10, 9,
    8, 7, 5, 4, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// `curveId` of the three edit values (`u32[0xC02BBF98 + 0x678..]`, VERIFIED by the run above).
pub const CURVE_ID: [u8; 3] = [10, 11, 13];

/// One of the three Touch curves, named by the firmware edit value `0 / 1 / 2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchCurve {
    /// Edit value 0, `curveId` 10.
    Heavy,
    /// Edit value 1, `curveId` 11 (the default of every program).
    Medium,
    /// Edit value 2, `curveId` 13.
    Light,
}

impl From<VelocityTouch> for TouchCurve {
    fn from(touch: VelocityTouch) -> Self {
        match touch {
            VelocityTouch::Heavy => Self::Heavy,
            VelocityTouch::Medium => Self::Medium,
            VelocityTouch::Light => Self::Light,
        }
    }
}

impl TouchCurve {
    /// The firmware edit value (`0..=2`).
    pub fn edit_value(self) -> u8 {
        match self {
            Self::Heavy => 0,
            Self::Medium => 1,
            Self::Light => 2,
        }
    }

    /// `T(v)` for `v = 1..=127` (index `v - 1`).
    pub fn table(self) -> &'static [u8; 127] {
        match self {
            Self::Heavy => &T_HEAVY,
            Self::Medium => &T_BY_VELOCITY,
            Self::Light => &T_LIGHT,
        }
    }

    /// Velocity position `T` of a MIDI velocity (clamped to `1..=127`).
    pub fn velocity_position(self, velocity: u8) -> u8 {
        self.table()[usize::from(velocity.clamp(1, 127)) - 1]
    }

    /// Level attenuation `X` of a MIDI velocity under this curve.
    pub fn x(self, velocity: u8) -> i16 {
        x_from_position(self.velocity_position(velocity))
    }

    /// Gain of this curve relative to the Medium curve at `velocity`: `2^(-2 (X - X_medium) / 60)`
    /// (60 level steps per octave, `n = ... + 2 X`, see [`super::level_q23`]). Used by the sounds
    /// that have no firmware level model, so that only the Touch difference is added.
    pub fn gain_vs_medium(self, velocity: u8) -> f32 {
        let dx = f32::from(self.x(velocity)) - f32::from(Self::Medium.x(velocity));
        2.0f32.powf(-2.0 * dx / 60.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [TouchCurve; 3] = [TouchCurve::Heavy, TouchCurve::Medium, TouchCurve::Light];

    /// FNV-1a over the three tables: a change of any byte fails this test.
    fn fingerprint() -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for curve in ALL {
            for &b in curve.table() {
                h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    #[test]
    fn table_fingerprint() {
        assert_eq!(
            fingerprint(),
            0xc8293ca1773436e4,
            "tables changed: re-run tools/arm_touch_curves.py"
        );
    }

    #[test]
    fn endpoints_and_anchors() {
        // (velocity, heavy, medium, light) read from the ARM run.
        let arm = [
            (1u8, 200u8, 200u8, 200u8),
            (2, 200, 200, 183),
            (10, 192, 178, 163),
            (33, 143, 131, 118),
            (64, 88, 78, 68),
            (90, 49, 40, 32),
            (100, 35, 27, 19),
            (120, 8, 2, 0),
            (127, 0, 0, 0),
        ];
        for (v, h, m, l) in arm {
            assert_eq!(TouchCurve::Heavy.velocity_position(v), h, "heavy v{v}");
            assert_eq!(TouchCurve::Medium.velocity_position(v), m, "medium v{v}");
            assert_eq!(TouchCurve::Light.velocity_position(v), l, "light v{v}");
        }
    }

    #[test]
    fn every_curve_is_non_increasing_and_ordered() {
        for curve in ALL {
            assert!(curve.table().windows(2).all(|w| w[0] >= w[1]), "{curve:?}");
        }
        for v in 1..=127u8 {
            let h = TouchCurve::Heavy.velocity_position(v);
            let m = TouchCurve::Medium.velocity_position(v);
            let l = TouchCurve::Light.velocity_position(v);
            assert!(h >= m && m >= l, "velocity {v}: {h} {m} {l}");
        }
    }

    #[test]
    fn velocity_is_clamped_and_medium_is_the_verified_default() {
        assert_eq!(TouchCurve::Light.velocity_position(0), 200);
        assert_eq!(TouchCurve::Light.velocity_position(255), 0);
        assert_eq!(TouchCurve::from(VelocityTouch::Medium), TouchCurve::Medium);
        assert_eq!(TouchCurve::from(VelocityTouch::Heavy), TouchCurve::Heavy);
        assert_eq!(TouchCurve::from(VelocityTouch::Light), TouchCurve::Light);
        assert_eq!(TouchCurve::Medium.edit_value(), 1);
        assert_eq!(TouchCurve::Heavy.edit_value(), 0);
        assert_eq!(TouchCurve::Light.edit_value(), 2);
        assert_eq!(CURVE_ID, [10, 11, 13]);
    }

    #[test]
    fn gain_is_unity_for_medium_and_follows_the_x_difference() {
        for v in 1..=127u8 {
            assert!((TouchCurve::Medium.gain_vs_medium(v) - 1.0).abs() < 1e-6);
            assert!(TouchCurve::Light.gain_vs_medium(v) >= 1.0);
            assert!(TouchCurve::Heavy.gain_vs_medium(v) <= 1.0);
        }
        // Velocity 64: X = (109 * T) >> 7 = 66 / 57 / 74 for T = 78 / 68 / 88.
        let want = 2.0f32.powf(-2.0 * (57.0 - 66.0) / 60.0);
        assert!((TouchCurve::Light.gain_vs_medium(64) - want).abs() < 1e-5);
    }

    #[test]
    fn touch_changes_the_level_input_x() {
        assert_eq!(TouchCurve::Medium.x(64), 66);
        assert_eq!(TouchCurve::Light.x(64), 57);
        assert_eq!(TouchCurve::Heavy.x(64), 74);
    }
}
