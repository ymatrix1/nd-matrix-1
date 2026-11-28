//! Deterministic fake LLE backend: loops every input event back out as telemetry.
//!
//! It implements both [`LleInput`] and [`LleTelemetry`] with no engine behind them, so the bridge
//! can be tested end to end before the real `LleEngine` (`ns4-lle5`) exists. Everything is
//! preallocated; nothing here allocates after [`FakeBackend::new`].
//!
//! Loopback mapping (a test convention, not firmware behaviour):
//! * `frame` = events handled so far,
//! * `oled[0][0..16]` = wire bytes of the last event, `oled[1][0..4]` = its block offset (LE),
//! * LED `note` (0 to 127) is lit while that note is held (`NoteOn` with velocity > 0 sets it,
//!   `NoteOff` clears it).

use nord_core::lle_io::{LleEvent, LleInput, LleTelemetry, TelemetrySnapshot, Wire};

/// Events the log keeps; later ones are counted in [`FakeBackend::lost`], not stored.
pub const LOG_CAPACITY: usize = 4096;

/// Loopback backend; see the module doc.
pub struct FakeBackend {
    log: Box<[(u32, LleEvent)]>,
    len: usize,
    lost: u64,
    resyncs: u32,
    last: Option<(u32, LleEvent)>,
    held: [bool; 128],
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    /// Allocates the log (the only allocation).
    pub fn new() -> Self {
        Self {
            log: vec![(0, LleEvent::Nop); LOG_CAPACITY].into_boxed_slice(),
            len: 0,
            lost: 0,
            resyncs: 0,
            last: None,
            held: [false; 128],
        }
    }

    /// Every event handled so far with its block offset, in arrival order (up to [`LOG_CAPACITY`]).
    pub fn log(&self) -> &[(u32, LleEvent)] {
        &self.log[..self.len]
    }

    /// Events that did not fit the log.
    pub fn lost(&self) -> u64 {
        self.lost
    }

    /// Times `resync_after_drop` ran.
    pub fn resyncs(&self) -> u32 {
        self.resyncs
    }

    /// Total events handled (stored or not).
    pub fn handled(&self) -> u64 {
        self.len as u64 + self.lost
    }
}

impl LleInput for FakeBackend {
    fn handle_event(&mut self, event: LleEvent) {
        self.handle_event_at(0, event);
    }

    fn handle_event_at(&mut self, offset: u32, event: LleEvent) {
        match event {
            LleEvent::NoteOn { note, velocity, .. } if velocity > 0 => {
                if let Some(h) = self.held.get_mut(note as usize) {
                    *h = true;
                }
            }
            LleEvent::NoteOn { note, .. } | LleEvent::NoteOff { note, .. } => {
                if let Some(h) = self.held.get_mut(note as usize) {
                    *h = false;
                }
            }
            _ => {}
        }
        if self.len < LOG_CAPACITY {
            self.log[self.len] = (offset, event);
            self.len += 1;
        } else {
            self.lost += 1;
        }
        self.last = Some((offset, event));
    }

    fn resync_after_drop(&mut self) {
        // Counted, not expanded to 64 events: the test cares that it ran once.
        self.resyncs += 1;
        self.held = [false; 128];
    }
}

impl LleTelemetry for FakeBackend {
    fn fill_telemetry(&self, out: &mut TelemetrySnapshot) {
        out.frame = self.handled() as u32;
        out.oled = [[0; nord_core::lle_io::telemetry::OLED_BYTES]; 2];
        if let Some((offset, event)) = self.last {
            out.oled[0][..16].copy_from_slice(&event.to_wire());
            out.oled[1][..4].copy_from_slice(&offset.to_le_bytes());
        }
        out.leds = [0; nord_core::lle_io::telemetry::LED_WORDS];
        for (n, on) in self.held.iter().enumerate() {
            if *on {
                out.set_led(n, true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_logged_in_order_with_offsets_and_echoed_as_telemetry() {
        let mut f = FakeBackend::new();
        let on = LleEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        };
        f.handle_event_at(17, on);
        f.handle_event_at(
            40,
            LleEvent::NoteOn {
                channel: 0,
                note: 64,
                velocity: 100,
            },
        );
        f.handle_event_at(
            90,
            LleEvent::NoteOff {
                channel: 0,
                note: 60,
                velocity: 0,
            },
        );
        assert_eq!(f.log()[0], (17, on));
        assert_eq!(
            f.log().iter().map(|e| e.0).collect::<Vec<_>>(),
            [17, 40, 90]
        );
        let mut snap = TelemetrySnapshot::default();
        f.fill_telemetry(&mut snap);
        assert_eq!(snap.frame, 3);
        assert!(!snap.led(60) && snap.led(64));
        assert_eq!(&snap.oled[1][..4], &90u32.to_le_bytes());
        assert_eq!(
            LleEvent::from_wire(snap.oled[0][..16].try_into().unwrap()),
            f.last.unwrap().1
        );
    }

    #[test]
    fn the_log_caps_and_counts_the_rest() {
        let mut f = FakeBackend::new();
        for _ in 0..(LOG_CAPACITY + 3) {
            f.handle_event(LleEvent::ProgramChange {
                channel: 0,
                program: 1,
            });
        }
        assert_eq!(
            (f.log().len(), f.lost(), f.handled()),
            (LOG_CAPACITY, 3, LOG_CAPACITY as u64 + 3)
        );
    }

    #[test]
    fn resync_clears_held_notes_and_is_counted() {
        let mut f = FakeBackend::new();
        f.handle_event(LleEvent::NoteOn {
            channel: 0,
            note: 5,
            velocity: 1,
        });
        f.resync_after_drop();
        let mut snap = TelemetrySnapshot::default();
        f.fill_telemetry(&mut snap);
        assert!(!snap.led(5));
        assert_eq!(f.resyncs(), 1);
    }
}
