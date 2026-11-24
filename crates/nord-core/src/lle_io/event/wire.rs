//! 16-byte wire form of [`LleEvent`] (the ring slot payload).
//!
//! Byte 0 is the tag (`0` = `Nop`, so an unwritten slot decodes to `Nop`); the rest are the
//! fields, little endian. Unknown tags and unknown pedal tags decode to `Nop`.

use super::{LleEvent, PanelEvent, Pedal};
use crate::lle_io::ring::Wire;

impl Wire for LleEvent {
    fn to_wire(self) -> [u8; 16] {
        let mut b = [0u8; 16];
        match self {
            Self::Nop => {}
            Self::NoteOn {
                channel,
                note,
                velocity,
            } => b[..4].copy_from_slice(&[1, channel, note, velocity]),
            Self::NoteOff {
                channel,
                note,
                velocity,
            } => b[..4].copy_from_slice(&[2, channel, note, velocity]),
            Self::ControlChange {
                channel,
                controller,
                value,
            } => b[..4].copy_from_slice(&[3, channel, controller, value]),
            Self::ChannelAftertouch { channel, pressure } => {
                b[..3].copy_from_slice(&[4, channel, pressure]);
            }
            Self::PitchBend { channel, value } => {
                b[..2].copy_from_slice(&[5, channel]);
                b[2..4].copy_from_slice(&value.to_le_bytes());
            }
            Self::ProgramChange { channel, program } => {
                b[..3].copy_from_slice(&[6, channel, program]);
            }
            Self::Pedal {
                channel,
                pedal,
                value,
            } => b[..4].copy_from_slice(&[7, channel, pedal.tag(), value]),
            Self::Panel(PanelEvent::Encoder { index, delta }) => {
                b[..2].copy_from_slice(&[8, index]);
                b[2..4].copy_from_slice(&delta.to_le_bytes());
            }
            Self::Panel(PanelEvent::Key { id, pressed }) => {
                b[..3].copy_from_slice(&[9, id, u8::from(pressed)]);
            }
            Self::Panel(PanelEvent::Analog { id, value }) => {
                b[0] = 10;
                b[1..3].copy_from_slice(&id.to_le_bytes());
                b[3..5].copy_from_slice(&value.to_le_bytes());
            }
        }
        b
    }

    fn from_wire(b: [u8; 16]) -> Self {
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        match b[0] {
            1 => Self::NoteOn {
                channel: b[1],
                note: b[2],
                velocity: b[3],
            },
            2 => Self::NoteOff {
                channel: b[1],
                note: b[2],
                velocity: b[3],
            },
            3 => Self::ControlChange {
                channel: b[1],
                controller: b[2],
                value: b[3],
            },
            4 => Self::ChannelAftertouch {
                channel: b[1],
                pressure: b[2],
            },
            5 => Self::PitchBend {
                channel: b[1],
                value: u16_at(2),
            },
            6 => Self::ProgramChange {
                channel: b[1],
                program: b[2],
            },
            7 => match Pedal::from_tag(b[2]) {
                Some(pedal) => Self::Pedal {
                    channel: b[1],
                    pedal,
                    value: b[3],
                },
                None => Self::Nop,
            },
            8 => Self::Panel(PanelEvent::Encoder {
                index: b[1],
                delta: u16_at(2) as i16,
            }),
            9 => Self::Panel(PanelEvent::Key {
                id: b[1],
                pressed: b[2] != 0,
            }),
            10 => Self::Panel(PanelEvent::Analog {
                id: u16_at(1),
                value: u16_at(3),
            }),
            _ => Self::Nop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::cc;

    fn all_variants() -> Vec<LleEvent> {
        vec![
            LleEvent::Nop,
            LleEvent::NoteOn {
                channel: 15,
                note: 127,
                velocity: 1,
            },
            LleEvent::NoteOff {
                channel: 0,
                note: 21,
                velocity: 64,
            },
            LleEvent::ControlChange {
                channel: 3,
                controller: cc::MOD_WHEEL,
                value: 99,
            },
            LleEvent::ChannelAftertouch {
                channel: 2,
                pressure: 77,
            },
            LleEvent::PitchBend {
                channel: 1,
                value: 16383,
            },
            LleEvent::ProgramChange {
                channel: 9,
                program: 5,
            },
            LleEvent::Pedal {
                channel: 0,
                pedal: Pedal::Sostenuto,
                value: 127,
            },
            LleEvent::Panel(PanelEvent::Encoder {
                index: 6,
                delta: -3,
            }),
            LleEvent::Panel(PanelEvent::Key {
                id: 21,
                pressed: true,
            }),
            LleEvent::Panel(PanelEvent::Analog {
                id: 513,
                value: 65535,
            }),
        ]
    }

    #[test]
    fn every_variant_round_trips_through_the_wire_form() {
        for e in all_variants() {
            assert_eq!(LleEvent::from_wire(e.to_wire()), e, "{e:?}");
        }
    }

    #[test]
    fn zeroed_slot_and_unknown_tag_decode_to_nop() {
        assert_eq!(LleEvent::from_wire([0; 16]), LleEvent::Nop);
        let mut b = [0u8; 16];
        b[0] = 200;
        assert_eq!(LleEvent::from_wire(b), LleEvent::Nop);
    }

    #[test]
    fn all_pedals_round_trip() {
        for p in [
            Pedal::Sustain,
            Pedal::Sostenuto,
            Pedal::Soft,
            Pedal::Control,
        ] {
            let e = LleEvent::Pedal {
                channel: 4,
                pedal: p,
                value: 10,
            };
            assert_eq!(LleEvent::from_wire(e.to_wire()), e);
        }
    }

    #[test]
    fn unknown_pedal_tag_decodes_to_nop() {
        let mut b = LleEvent::Pedal {
            channel: 0,
            pedal: Pedal::Sustain,
            value: 127,
        }
        .to_wire();
        b[2] = 9;
        assert_eq!(LleEvent::from_wire(b), LleEvent::Nop);
    }
}
