//! Engine-neutral live input events for the LLE backend (bean `ns4-lle6`).
//!
//! One `Copy` enum covers live MIDI (re-using [`crate::midi::MidiEvent`] fields), pedals and panel
//! edits. It packs into 16 bytes ([`Wire`]) so it travels through the wait-free ring unchanged.
//! The future `LleEngine` turns each event into ARM0 input-queue traffic; nothing here knows the
//! ARM0 side.

mod wire;

use crate::midi::{cc, MidiEvent};

/// Foot pedal (or pedal-like controller) the firmware treats separately from plain CCs.
///
/// ASSUMED: the CC 64 / 66 / 67 / 11 to pedal mapping follows `midi::cc`; how the ARM0 input path
/// consumes pedals is unknown until `ns4-lle5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pedal {
    /// CC 64.
    Sustain,
    /// CC 66.
    Sostenuto,
    /// CC 67.
    Soft,
    /// CC 11 (`midi::cc::CONTROL_PEDAL`). ASSUMED: the engine already names CC 11 the control
    /// pedal; standard MIDI calls it Expression. Confirm against the firmware's own CC table when
    /// `ns4-lle5` can inject it.
    Control,
}

impl Pedal {
    /// Pedal a MIDI controller number stands for, if any.
    pub fn from_cc(controller: u8) -> Option<Self> {
        match controller {
            cc::SUSTAIN_PEDAL => Some(Self::Sustain),
            cc::SOSTENUTO_PEDAL => Some(Self::Sostenuto),
            cc::SOFT_PEDAL => Some(Self::Soft),
            cc::CONTROL_PEDAL => Some(Self::Control),
            _ => None,
        }
    }

    pub(super) fn tag(self) -> u8 {
        self as u8
    }

    /// `None` for an unknown tag, so a corrupt slot decodes to `Nop`, not a sustain press.
    pub(super) fn from_tag(t: u8) -> Option<Self> {
        match t {
            0 => Some(Self::Sustain),
            1 => Some(Self::Sostenuto),
            2 => Some(Self::Soft),
            3 => Some(Self::Control),
            _ => None,
        }
    }
}

/// Front-panel edit, addressed by the firmware's own ids.
///
/// VERIFIED: encoder `index` and key `id` are the ids the ARM0 scan decoder posts as UI events
/// (`{tag, source, encoder index, delta}` and `{tag, 0, key id, 1 / 0}`), source
/// `tools/arm_events.py` module doc, `docs/research/arm-oled-render.md` s15 (Program dial = index
/// 6, PIANO SELECT = 7, Synth dials = 15, 14, 13); checked by `tools/arm_events.py turn`, not by
/// a Rust test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelEvent {
    /// Endless dial turned by `delta` detents (negative = counter-clockwise).
    Encoder { index: u8, delta: i16 },
    /// Button or key `id` pressed (`true`) or released.
    Key { id: u8, pressed: bool },
    /// Analog control (knob, fader, drawbar, wheel, pitch stick) at `value`, `0..=65535` of its
    /// travel. ASSUMED: the firmware's ADC scale is applied later by the engine wiring
    /// (`ns4-lle5`); full-scale u16 keeps this layer scale-free.
    Analog { id: u16, value: u16 },
}

/// One live input event for the LLE engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LleEvent {
    /// Empty slot (also what an unknown wire tag decodes to).
    #[default]
    Nop,
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    ChannelAftertouch {
        channel: u8,
        pressure: u8,
    },
    /// 14-bit pitch bend, `0..=16383`, centre `8192`.
    PitchBend {
        channel: u8,
        value: u16,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    Pedal {
        channel: u8,
        pedal: Pedal,
        value: u8,
    },
    Panel(PanelEvent),
}

impl From<MidiEvent> for LleEvent {
    /// Pedal controllers (64, 66, 67, 11) become [`LleEvent::Pedal`]; every other CC stays a CC.
    fn from(e: MidiEvent) -> Self {
        match e {
            MidiEvent::NoteOn {
                channel,
                note,
                velocity,
            } => Self::NoteOn {
                channel,
                note,
                velocity,
            },
            MidiEvent::NoteOff {
                channel,
                note,
                velocity,
            } => Self::NoteOff {
                channel,
                note,
                velocity,
            },
            MidiEvent::ControlChange {
                channel,
                controller,
                value,
            } => match Pedal::from_cc(controller) {
                Some(pedal) => Self::Pedal {
                    channel,
                    pedal,
                    value,
                },
                None => Self::ControlChange {
                    channel,
                    controller,
                    value,
                },
            },
            MidiEvent::ChannelAftertouch { channel, pressure } => {
                Self::ChannelAftertouch { channel, pressure }
            }
            MidiEvent::ProgramChange { channel, program } => {
                Self::ProgramChange { channel, program }
            }
            MidiEvent::PitchBend { channel, value } => Self::PitchBend { channel, value },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_pedal_ccs_become_pedal_events_and_others_stay_ccs() {
        let cc_ev = |controller| MidiEvent::ControlChange {
            channel: 2,
            controller,
            value: 100,
        };
        for (c, p) in [
            (cc::SUSTAIN_PEDAL, Pedal::Sustain),
            (cc::SOSTENUTO_PEDAL, Pedal::Sostenuto),
            (cc::SOFT_PEDAL, Pedal::Soft),
            (cc::CONTROL_PEDAL, Pedal::Control),
        ] {
            assert_eq!(
                LleEvent::from(cc_ev(c)),
                LleEvent::Pedal {
                    channel: 2,
                    pedal: p,
                    value: 100
                }
            );
        }
        assert_eq!(
            LleEvent::from(cc_ev(cc::MOD_WHEEL)),
            LleEvent::ControlChange {
                channel: 2,
                controller: cc::MOD_WHEEL,
                value: 100
            }
        );
    }

    #[test]
    fn every_midi_event_kind_converts() {
        let n = LleEvent::from(MidiEvent::NoteOn {
            channel: 1,
            note: 60,
            velocity: 90,
        });
        assert!(matches!(n, LleEvent::NoteOn { note: 60, .. }));
        assert_eq!(
            LleEvent::from(MidiEvent::PitchBend {
                channel: 0,
                value: 8192
            }),
            LleEvent::PitchBend {
                channel: 0,
                value: 8192
            }
        );
        assert_eq!(
            LleEvent::from(MidiEvent::ProgramChange {
                channel: 0,
                program: 7
            }),
            LleEvent::ProgramChange {
                channel: 0,
                program: 7
            }
        );
        assert_eq!(
            LleEvent::from(MidiEvent::ChannelAftertouch {
                channel: 0,
                pressure: 5
            }),
            LleEvent::ChannelAftertouch {
                channel: 0,
                pressure: 5
            }
        );
        assert!(matches!(
            LleEvent::from(MidiEvent::NoteOff {
                channel: 0,
                note: 1,
                velocity: 0
            }),
            LleEvent::NoteOff { note: 1, .. }
        ));
    }
}
