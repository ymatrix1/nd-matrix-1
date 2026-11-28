//! Ring items that carry the host sample offset next to the neutral event.
//!
//! `lle_io::LleEvent` has no time stamp, but the host delivers MIDI with a sample offset inside the
//! block. [`Stamped`] packs both into the same 16-byte ring slot: the event keeps its own wire
//! bytes (0 to 4 are used by every variant) and the offset sits in bytes 12 to 15.

use nord_core::lle_io::{Consumer, LleEvent, LleInput, Wire, MAX_EVENTS_PER_DRAIN};

/// An event and its sample offset inside the audio block it arrived in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamped {
    /// Samples from the start of the block (0 for panel edits, which have no host time stamp).
    pub offset: u32,
    pub event: LleEvent,
}

/// Consumer end of the stamped event ring.
pub type StampedConsumer = Consumer<Stamped>;

/// First byte of the offset in the 16-byte slot.
const OFFSET_AT: usize = 12;

impl Wire for Stamped {
    fn to_wire(self) -> [u8; 16] {
        let mut b = self.event.to_wire();
        debug_assert!(
            b[OFFSET_AT..].iter().all(|x| *x == 0),
            "event grew into the offset bytes"
        );
        b[OFFSET_AT..].copy_from_slice(&self.offset.to_le_bytes());
        b
    }

    fn from_wire(mut b: [u8; 16]) -> Self {
        let offset = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
        b[OFFSET_AT..].fill(0);
        Self {
            offset,
            event: LleEvent::from_wire(b),
        }
    }
}

/// [`LleInput::drain`] for stamped events: applies up to [`MAX_EVENTS_PER_DRAIN`] events in order
/// through `handle_event_at`, skips `Nop`, and runs `resync_after_drop` once after the queue
/// empties if the producer lost events. Returns how many queue items it consumed. Allocation- and
/// lock-free.
pub fn drain_stamped<I: LleInput + ?Sized>(input: &mut I, rx: &mut StampedConsumer) -> usize {
    let mut n = 0;
    while n < MAX_EVENTS_PER_DRAIN {
        match rx.pop() {
            Some(Stamped {
                event: LleEvent::Nop,
                ..
            }) => {}
            Some(s) => input.handle_event_at(s.offset, s.event),
            None => {
                if rx.take_new_drops() > 0 {
                    input.resync_after_drop();
                }
                break;
            }
        }
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use nord_core::lle_io::ring::channel;
    use nord_core::lle_io::{PanelEvent, Pedal};

    fn every_variant() -> [LleEvent; 11] {
        [
            LleEvent::Nop,
            LleEvent::NoteOn {
                channel: 15,
                note: 127,
                velocity: 127,
            },
            LleEvent::NoteOff {
                channel: 1,
                note: 0,
                velocity: 64,
            },
            LleEvent::ControlChange {
                channel: 2,
                controller: 7,
                value: 99,
            },
            LleEvent::ChannelAftertouch {
                channel: 3,
                pressure: 80,
            },
            LleEvent::PitchBend {
                channel: 4,
                value: 16383,
            },
            LleEvent::ProgramChange {
                channel: 5,
                program: 127,
            },
            LleEvent::Pedal {
                channel: 6,
                pedal: Pedal::Control,
                value: 127,
            },
            LleEvent::Panel(PanelEvent::Encoder {
                index: 255,
                delta: -32768,
            }),
            LleEvent::Panel(PanelEvent::Key {
                id: 255,
                pressed: true,
            }),
            LleEvent::Panel(PanelEvent::Analog {
                id: 65535,
                value: 65535,
            }),
        ]
    }

    #[test]
    fn wire_round_trip_keeps_event_and_offset_for_every_variant() {
        for event in every_variant() {
            for offset in [0, 1, 511, u32::MAX] {
                let s = Stamped { offset, event };
                assert_eq!(Stamped::from_wire(s.to_wire()), s, "{event:?} @ {offset}");
            }
        }
    }

    #[test]
    fn an_unwritten_slot_is_a_nop_at_offset_zero() {
        assert_eq!(
            Stamped::from_wire([0; 16]),
            Stamped {
                offset: 0,
                event: LleEvent::Nop
            }
        );
    }

    #[test]
    fn no_event_uses_the_offset_bytes() {
        for event in every_variant() {
            assert!(
                event.to_wire()[OFFSET_AT..].iter().all(|b| *b == 0),
                "{event:?}"
            );
        }
    }

    #[derive(Default)]
    struct Rec {
        seen: Vec<(u32, LleEvent)>,
        resyncs: u32,
    }

    impl LleInput for Rec {
        fn handle_event(&mut self, event: LleEvent) {
            self.seen.push((u32::MAX, event));
        }
        fn handle_event_at(&mut self, offset: u32, event: LleEvent) {
            self.seen.push((offset, event));
        }
        fn resync_after_drop(&mut self) {
            self.resyncs += 1;
        }
    }

    #[test]
    fn drain_delivers_offsets_in_order_skips_nops_and_resyncs_once_after_a_drop() {
        let (mut tx, mut rx) = channel::<Stamped>(4);
        let on = |note| LleEvent::NoteOn {
            channel: 0,
            note,
            velocity: 90,
        };
        assert!(tx.push(Stamped {
            offset: 3,
            event: on(60)
        }));
        assert!(tx.push(Stamped {
            offset: 9,
            event: LleEvent::Nop
        }));
        assert!(tx.push(Stamped {
            offset: 9,
            event: on(62)
        }));
        assert!(tx.push(Stamped {
            offset: 100,
            event: on(64)
        }));
        assert!(
            !tx.push(Stamped {
                offset: 101,
                event: on(65)
            }),
            "ring full"
        );
        let mut r = Rec::default();
        assert_eq!(drain_stamped(&mut r, &mut rx), 4);
        assert_eq!(r.seen, [(3, on(60)), (9, on(62)), (100, on(64))]);
        assert_eq!(r.resyncs, 1);
        assert_eq!(drain_stamped(&mut r, &mut rx), 0);
        assert_eq!(r.resyncs, 1, "one resync per loss");
    }

    #[test]
    fn drain_is_bounded_per_call() {
        let (mut tx, mut rx) = channel::<Stamped>(1024);
        for i in 0..(MAX_EVENTS_PER_DRAIN as u32 + 5) {
            assert!(tx.push(Stamped {
                offset: i,
                event: LleEvent::ProgramChange {
                    channel: 0,
                    program: 1
                }
            }));
        }
        let mut r = Rec::default();
        assert_eq!(drain_stamped(&mut r, &mut rx), MAX_EVENTS_PER_DRAIN);
        assert_eq!(drain_stamped(&mut r, &mut rx), 5);
    }
}
