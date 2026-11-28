//! Engine-neutral live I/O for the LLE backend (epic `ns4-lle1`, bean `ns4-lle6`).
//!
//! Everything the future `LleEngine` (`ns4-lle5`) needs to be played and watched, without knowing
//! ARM0 or SHARC: wait-free input rings ([`ring`]), live MIDI / pedal / panel events ([`event`]),
//! the GUI and MIDI translation layer ([`translate`]), lock-free OLED / LED telemetry
//! ([`telemetry`], over [`seqlock`]) and the traits the engine implements ([`LleInput`],
//! [`LleTelemetry`]). Spec: `docs/specs/lle-io.md`.
//!
//! Real-time rule (`.agents/rules/dsp-and-realtime.md`): every `push`, `pop`, `publish` and `read`
//! here is allocation-free and lock-free; rings and cells are built once by [`link`].

pub mod event;
pub mod ring;
pub mod seqlock;
pub mod telemetry;
pub mod translate;

pub use event::{LleEvent, PanelEvent, Pedal};
pub use ring::{Consumer, Producer, Wire};
pub use telemetry::{TelemetryReader, TelemetrySnapshot, TelemetryWriter};
pub use translate::{midi_bytes, panel_edit, PanelKind};

/// Producer end of the event ring (host side).
pub type EventProducer = Producer<LleEvent>;
/// Consumer end of the event ring (engine side).
pub type EventConsumer = Consumer<LleEvent>;

/// Events one [`LleInput::drain`] call applies at most, so a flooded ring cannot stall a block.
/// ASSUMED: 256 is far above a realistic burst per 5 to 10 ms audio buffer (a dense chord plus
/// controllers is under 50); the cap only bounds the worst case.
pub const MAX_EVENTS_PER_DRAIN: usize = 256;

/// Host-side ends: feed input, watch telemetry.
pub struct HostEnds {
    pub events: EventProducer,
    pub telemetry: TelemetryReader,
}

/// Engine-side ends: take input, publish telemetry.
pub struct EngineEnds {
    pub events: EventConsumer,
    pub telemetry: TelemetryWriter,
}

/// Builds both ends of the link; `event_capacity` is rounded up to a power of two. Allocates here,
/// at init, never later.
pub fn link(event_capacity: usize) -> (HostEnds, EngineEnds) {
    let (tx, rx) = ring::channel(event_capacity);
    let (tw, tr) = telemetry::channel();
    (
        HostEnds {
            events: tx,
            telemetry: tr,
        },
        EngineEnds {
            events: rx,
            telemetry: tw,
        },
    )
}

/// Input side of an LLE engine: turns neutral events into ARM0 input-queue traffic.
pub trait LleInput {
    /// Applies one event (the engine writes it into the ARM0 input queues). Real-time safe.
    fn handle_event(&mut self, event: LleEvent);

    /// Applies one event that happened `offset` samples into the current audio block (the host
    /// MIDI timestamp). The default ignores the offset; an engine that renders sample-accurately
    /// overrides it. Real-time safe.
    fn handle_event_at(&mut self, _offset: u32, event: LleEvent) {
        self.handle_event(event);
    }

    /// Applies up to [`MAX_EVENTS_PER_DRAIN`] queued events in order; returns how many queued
    /// events it consumed.
    ///
    /// A full ring rejects the newest event (`NoteOff` and pedal release included), which would
    /// leave notes hanging. Once the queue is empty, if the producer lost events since the last
    /// drain, [`Self::resync_after_drop`] runs (after the queued events, so a surviving `NoteOn`
    /// is cut rather than a lost `NoteOff` left sounding).
    fn drain(&mut self, rx: &mut EventConsumer) -> usize {
        let mut n = 0;
        while n < MAX_EVENTS_PER_DRAIN {
            match rx.pop() {
                Some(LleEvent::Nop) => {}
                Some(e) => self.handle_event(e),
                None => {
                    if rx.take_new_drops() > 0 {
                        self.resync_after_drop();
                    }
                    break;
                }
            }
            n += 1;
        }
        n
    }

    /// Releases everything a dropped event may have left held: all-notes-off and sustain /
    /// sostenuto / soft pedal up on all 16 channels. The continuous control pedal is left alone
    /// (it has no "release"). Override if the engine has a cheaper panic path.
    fn resync_after_drop(&mut self) {
        for channel in 0..16 {
            self.handle_event(LleEvent::ControlChange {
                channel,
                controller: crate::midi::cc::ALL_NOTES_OFF,
                value: 0,
            });
            for pedal in [Pedal::Sustain, Pedal::Sostenuto, Pedal::Soft] {
                self.handle_event(LleEvent::Pedal {
                    channel,
                    pedal,
                    value: 0,
                });
            }
        }
    }
}

/// Output side of an LLE engine: reports what the emulated panel shows.
pub trait LleTelemetry {
    /// Fills `out` from the current OLED framebuffers and LED state. Real-time safe.
    fn fill_telemetry(&self, out: &mut TelemetrySnapshot);

    /// Fills `scratch` and publishes it; `scratch` is caller-owned so nothing allocates here.
    fn publish_telemetry(&self, scratch: &mut TelemetrySnapshot, w: &mut TelemetryWriter) {
        self.fill_telemetry(scratch);
        w.publish(scratch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Mock {
        seen: Vec<LleEvent>,
        frame: u32,
    }

    impl LleInput for Mock {
        fn handle_event(&mut self, e: LleEvent) {
            self.seen.push(e);
        }
    }

    impl LleTelemetry for Mock {
        fn fill_telemetry(&self, out: &mut TelemetrySnapshot) {
            out.frame = self.frame;
            out.set_led(3, true);
        }
    }

    #[test]
    fn host_events_reach_the_engine_in_order_and_nops_are_skipped() {
        let (mut host, mut eng) = link(8);
        let notes = [
            midi_bytes(&[0x90, 60, 100]).unwrap(),
            midi_bytes(&[0xB0, 64, 127]).unwrap(),
            panel_edit(PanelKind::Button, 21, 1.0).unwrap(),
        ];
        for e in notes {
            assert!(host.events.push(e));
        }
        assert!(host.events.push(LleEvent::Nop));
        let mut m = Mock::default();
        assert_eq!(m.drain(&mut eng.events), 4);
        assert_eq!(m.seen, notes);
        assert_eq!(m.drain(&mut eng.events), 0);
    }

    #[test]
    fn handle_event_at_defaults_to_handle_event() {
        let mut m = Mock::default();
        let e = LleEvent::ProgramChange {
            channel: 1,
            program: 9,
        };
        m.handle_event_at(77, e);
        assert_eq!(m.seen, [e]);
    }

    #[test]
    fn drain_is_bounded_per_call() {
        let (mut host, mut eng) = link(1024);
        for _ in 0..(MAX_EVENTS_PER_DRAIN + 10) {
            assert!(host.events.push(LleEvent::ProgramChange {
                channel: 0,
                program: 1
            }));
        }
        let mut m = Mock::default();
        assert_eq!(m.drain(&mut eng.events), MAX_EVENTS_PER_DRAIN);
        assert_eq!(m.drain(&mut eng.events), 10);
    }

    /// A full ring drops the NoteOff; the engine must be told to release instead of hanging.
    #[test]
    fn a_dropped_event_triggers_one_resync_all_notes_and_pedals_off() {
        let (mut host, mut eng) = link(2);
        let on = |note| LleEvent::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        };
        assert!(host.events.push(on(60)) && host.events.push(on(64)));
        let off = LleEvent::NoteOff {
            channel: 0,
            note: 60,
            velocity: 0,
        };
        assert!(!host.events.push(off), "ring full: the NoteOff is lost");
        let mut m = Mock::default();
        assert_eq!(m.drain(&mut eng.events), 2);
        assert_eq!(&m.seen[..2], &[on(60), on(64)]);
        let tail = &m.seen[2..];
        assert_eq!(tail.len(), 16 * 4);
        assert!(tail.contains(&LleEvent::ControlChange {
            channel: 15,
            controller: crate::midi::cc::ALL_NOTES_OFF,
            value: 0
        }));
        assert!(tail.contains(&LleEvent::Pedal {
            channel: 0,
            pedal: Pedal::Sustain,
            value: 0
        }));
        assert!(!tail.iter().any(|e| matches!(
            e,
            LleEvent::Pedal {
                pedal: Pedal::Control,
                ..
            }
        )));
        let before = m.seen.len();
        assert_eq!(m.drain(&mut eng.events), 0);
        assert_eq!(m.seen.len(), before, "one resync per loss, not per drain");
    }

    #[test]
    fn engine_telemetry_reaches_the_host() {
        let (host, mut eng) = link(8);
        let m = Mock {
            frame: 42,
            ..Mock::default()
        };
        let mut scratch = TelemetrySnapshot::default();
        m.publish_telemetry(&mut scratch, &mut eng.telemetry);
        let mut out = TelemetrySnapshot::default();
        assert!(host.telemetry.read(&mut out));
        assert_eq!(out.frame, 42);
        assert!(out.led(3));
    }
}
