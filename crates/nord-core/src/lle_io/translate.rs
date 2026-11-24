//! Translation layer from the existing GUI / MIDI inputs into [`LleEvent`]s (`MidiToEvent` and
//! the panel side).
//!
//! MIDI: raw bytes -> [`MidiEvent::from_bytes`] -> [`LleEvent`]; the parser already folds
//! `NoteOn velocity 0` into `NoteOff`. Panel: the GUI's control kinds (`nord_panel::ControlKind`,
//! not imported here: `nord-core` stays free of the UI crate) are mapped to the neutral
//! [`PanelKind`], then to a [`PanelEvent`].

use super::event::{LleEvent, PanelEvent};
use crate::midi::MidiEvent;

/// Physical class of a panel control; mirrors `nord_panel::ControlKind` for the kinds that reach
/// the ARM0 (LEDs, OLEDs and the keybed notes do not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Knob,
    Fader,
    Drawbar,
    Wheel,
    Stick,
    Button,
    Encoder,
}

/// Parses one raw MIDI channel message (2 or 3 bytes) into an event; `None` for system messages
/// and malformed input.
pub fn midi_bytes(bytes: &[u8]) -> Option<LleEvent> {
    MidiEvent::from_bytes(bytes).map(LleEvent::from)
}

/// Normalises a `0.0..=1.0` GUI control value to the full-scale `u16` of [`PanelEvent::Analog`]
/// (clamped; NaN maps to 0).
pub fn unit_to_u16(v: f32) -> u16 {
    if v.is_nan() {
        return 0;
    }
    (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
}

/// Panel edit from the GUI: `id` is the firmware control id, `value` the GUI value.
///
/// * analog kinds: `value` is `0.0..=1.0` of travel, `id` is the full `u16`;
/// * `Button`: `value >= 0.5` means pressed; `id` must fit the firmware's `u8` key id;
/// * `Encoder`: `value` is the signed detent count (rounded, saturating at the `i16` range);
///   `id` must fit the firmware's `u8` encoder index.
///
/// `None` when a `Button` / `Encoder` id exceeds 255 (it would alias another control).
pub fn panel_edit(kind: PanelKind, id: u16, value: f32) -> Option<LleEvent> {
    let id8 = u8::try_from(id);
    Some(LleEvent::Panel(match kind {
        PanelKind::Knob
        | PanelKind::Fader
        | PanelKind::Drawbar
        | PanelKind::Wheel
        | PanelKind::Stick => PanelEvent::Analog {
            id,
            value: unit_to_u16(value),
        },
        PanelKind::Button => PanelEvent::Key {
            id: id8.ok()?,
            pressed: value >= 0.5,
        },
        PanelKind::Encoder => PanelEvent::Encoder {
            index: id8.ok()?,
            delta: if value.is_nan() {
                0
            } else {
                value
                    .round()
                    .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
            },
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lle_io::event::Pedal;

    #[test]
    fn raw_midi_translates_every_supported_kind_and_maps_pedal_ccs() {
        assert_eq!(
            midi_bytes(&[0x90, 60, 100]),
            Some(LleEvent::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100
            })
        );
        assert!(matches!(
            midi_bytes(&[0x90, 60, 0]),
            Some(LleEvent::NoteOff { note: 60, .. })
        ));
        assert_eq!(
            midi_bytes(&[0xB1, 64, 127]),
            Some(LleEvent::Pedal {
                channel: 1,
                pedal: Pedal::Sustain,
                value: 127
            })
        );
        assert_eq!(
            midi_bytes(&[0xE0, 0, 0x40]),
            Some(LleEvent::PitchBend {
                channel: 0,
                value: 8192
            })
        );
        assert_eq!(
            midi_bytes(&[0xC0, 12]),
            Some(LleEvent::ProgramChange {
                channel: 0,
                program: 12
            })
        );
        assert_eq!(
            midi_bytes(&[0xD0, 50]),
            Some(LleEvent::ChannelAftertouch {
                channel: 0,
                pressure: 50
            })
        );
    }

    #[test]
    fn system_and_truncated_midi_are_rejected() {
        assert_eq!(midi_bytes(&[]), None);
        assert_eq!(midi_bytes(&[0xF8]), None);
        assert_eq!(midi_bytes(&[0x90, 60]), None);
    }

    #[test]
    fn unit_scale_is_clamped_and_rounded() {
        assert_eq!(unit_to_u16(0.0), 0);
        assert_eq!(unit_to_u16(1.0), 65535);
        assert_eq!(unit_to_u16(2.0), 65535);
        assert_eq!(unit_to_u16(-1.0), 0);
        assert_eq!(unit_to_u16(f32::NAN), 0);
        assert_eq!(unit_to_u16(0.5), 32768);
    }

    fn panel(kind: PanelKind, id: u16, value: f32) -> PanelEvent {
        match panel_edit(kind, id, value) {
            Some(LleEvent::Panel(p)) => p,
            other => panic!("expected a panel event, got {other:?}"),
        }
    }

    #[test]
    fn panel_edits_map_by_kind() {
        assert_eq!(
            panel(PanelKind::Drawbar, 300, 1.0),
            PanelEvent::Analog {
                id: 300,
                value: 65535
            }
        );
        assert_eq!(
            panel(PanelKind::Knob, 7, 0.0),
            PanelEvent::Analog { id: 7, value: 0 }
        );
        assert_eq!(
            panel(PanelKind::Button, 21, 1.0),
            PanelEvent::Key {
                id: 21,
                pressed: true
            }
        );
        assert_eq!(
            panel(PanelKind::Button, 21, 0.0),
            PanelEvent::Key {
                id: 21,
                pressed: false
            }
        );
        assert_eq!(
            panel(PanelKind::Encoder, 6, -3.0),
            PanelEvent::Encoder {
                index: 6,
                delta: -3
            }
        );
        assert_eq!(
            panel(PanelKind::Encoder, 6, 1e9),
            PanelEvent::Encoder {
                index: 6,
                delta: i16::MAX
            }
        );
        assert_eq!(
            panel(PanelKind::Encoder, 6, f32::NAN),
            PanelEvent::Encoder { index: 6, delta: 0 }
        );
        for k in [PanelKind::Fader, PanelKind::Wheel, PanelKind::Stick] {
            assert!(matches!(
                panel(k, 1, 0.5),
                PanelEvent::Analog { value: 32768, .. }
            ));
        }
    }

    /// Regression (review of PR #650): `id as u8` made id 277 press key 21.
    #[test]
    fn button_and_encoder_ids_above_255_are_rejected_not_aliased() {
        assert_eq!(panel_edit(PanelKind::Button, 277, 1.0), None);
        assert_eq!(panel_edit(PanelKind::Encoder, 256, 1.0), None);
        assert!(panel_edit(PanelKind::Button, 255, 1.0).is_some());
        assert!(panel_edit(PanelKind::Encoder, 255, 1.0).is_some());
        // Analog ids use the full u16, so 277 is a valid knob.
        assert_eq!(
            panel(PanelKind::Knob, 277, 1.0),
            PanelEvent::Analog {
                id: 277,
                value: 65535
            }
        );
    }
}
