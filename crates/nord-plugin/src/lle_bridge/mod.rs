//! Opt-in bridge from the plugin's live I/O to the engine-neutral `lle_io` layer (bean `ns4-lle6`).
//!
//! Off by default: then every method returns at once and the plugin's audio path is the one it
//! always had. When the user asks for the LLE engine (`NS4_ENGINE=lle`, or the saved
//! `engine = "lle"` of the Settings tab, same precedence as `engine_select::requested`) and an
//! LLE engine exists, the bridge forwards host MIDI and panel / parameter edits into a wait-free
//! SPSC ring of [`Stamped`] events and lets the engine side publish OLED / LED telemetry back
//! through a seqlock the editor reads without locking. Without an engine (every build today:
//! `LleBackend::probe` is a stub) the bridge reports [`LLE_UNAVAILABLE`] and forwards nothing, so
//! the normal engine keeps running unchanged. Spec: `docs/specs/lle-io.md`.
//!
//! Real-time rule: [`LleBridge::forward_host_event`], [`LleBridge::scan_params`] and
//! [`LleBridge::end_block`] never allocate, lock or block; a full ring is counted
//! ([`LleBridge::overflow_count`]), not waited on. Rings are built by [`LleBridge::active`] at
//! initialisation.

pub mod fake;
pub mod stamped;

use crate::midi::to_core_midi_event;
use crate::params::Stage4Params;
use nih_plug::prelude::{NoteEvent, ParamPtr, Params};
use nord_core::engine_select::{self, EngineKind, LleAvailability, LleBackend};
use nord_core::lle_io::ring::channel;
use nord_core::lle_io::telemetry::{self, TelemetryReader, TelemetrySnapshot, TelemetryWriter};
use nord_core::lle_io::{panel_edit, LleEvent, LleInput, LleTelemetry, PanelKind, Producer};
pub use stamped::{drain_stamped, Stamped, StampedConsumer};

/// Text shown in place of the engine state when LLE was requested but cannot run.
pub const LLE_UNAVAILABLE: &str = "LLE engine not available in this build";

/// Ring slots between the plugin and the engine. ASSUMED: 1024 is far above one audio block of
/// MIDI plus changed parameters (a dense block is under 100); the ring only bounds the worst case.
pub const RING_CAPACITY: usize = 1024;

/// What the engine side must provide: take events, publish telemetry, move to the audio thread.
pub trait LleBackendIo: LleInput + LleTelemetry + Send {}
impl<T: LleInput + LleTelemetry + Send> LleBackendIo for T {}

/// Engine-side ends of an active bridge.
pub struct StampedEnds {
    pub events: StampedConsumer,
    pub telemetry: TelemetryWriter,
}

/// Where the bridge stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeState {
    /// Not requested: the plugin behaves exactly as without the bridge.
    Off,
    /// Requested but no LLE engine in this build: nothing is forwarded, the normal engine runs.
    Unavailable,
    /// Forwarding into the ring.
    Active,
}

/// The in-callback pump: engine ends, backend, and the telemetry scratch it publishes from.
struct Pump {
    ends: StampedEnds,
    backend: Box<dyn LleBackendIo>,
    scratch: Box<TelemetrySnapshot>,
}

/// The bridge; one per plugin instance.
pub struct LleBridge {
    state: BridgeState,
    tx: Option<Producer<Stamped>>,
    reader: Option<TelemetryReader>,
    /// Engine ends waiting for [`Self::take_engine_ends`] or [`Self::attach_backend`].
    ends: Option<StampedEnds>,
    pump: Option<Pump>,
    watched: Vec<ParamPtr>,
    shadow: Vec<f32>,
}

impl Default for LleBridge {
    fn default() -> Self {
        Self::off()
    }
}

impl LleBridge {
    /// A disabled bridge (the default).
    pub fn off() -> Self {
        Self {
            state: BridgeState::Off,
            tx: None,
            reader: None,
            ends: None,
            pump: None,
            watched: Vec::new(),
            shadow: Vec::new(),
        }
    }

    /// Decides from the environment value and the saved setting (`engine_select::requested`: the
    /// environment wins). An unknown value, or HLE, leaves the bridge off.
    pub fn from_settings(env: Option<&str>, saved: Option<&str>) -> Self {
        Self::from_settings_with(env, saved, LleBackend::probe)
    }

    /// [`Self::from_settings`] with the availability probe given (tests; the real engine later).
    pub fn from_settings_with(
        env: Option<&str>,
        saved: Option<&str>,
        probe: impl FnOnce() -> LleAvailability,
    ) -> Self {
        match engine_select::requested(&[], env, saved) {
            Ok(EngineKind::Lle) => match probe() {
                LleAvailability::Available => Self::active(RING_CAPACITY),
                LleAvailability::Unavailable(_) => Self {
                    state: BridgeState::Unavailable,
                    ..Self::off()
                },
            },
            _ => Self::off(),
        }
    }

    /// [`Self::from_settings`] from `NS4_ENGINE` and the saved config file. Reads a file: call it
    /// from `initialize`, never from `process`.
    pub fn from_process_settings() -> Self {
        let saved = nord_core::user_config::default_config_path()
            .and_then(|p| nord_core::user_config::load(&p).ok())
            .and_then(|c| c.engine);
        let env = std::env::var(engine_select::ENGINE_ENV).ok();
        Self::from_settings(env.as_deref(), saved.as_deref())
    }

    /// An active bridge with `capacity` ring slots; allocates here, once. The engine ends are
    /// returned for an engine thread, or call [`Self::attach_backend`] to pump from the callback.
    pub fn active(capacity: usize) -> Self {
        let (tx, rx) = channel::<Stamped>(capacity);
        let (tw, tr) = telemetry::channel();
        Self {
            state: BridgeState::Active,
            tx: Some(tx),
            reader: Some(tr),
            ends: Some(StampedEnds {
                events: rx,
                telemetry: tw,
            }),
            ..Self::off()
        }
    }

    pub fn state(&self) -> BridgeState {
        self.state
    }

    /// `true` when the audio callback has anything to do.
    pub fn is_active(&self) -> bool {
        self.state == BridgeState::Active
    }

    /// One line for the editor: `None` when off.
    pub fn status_text(&self) -> Option<&'static str> {
        match self.state {
            BridgeState::Off => None,
            BridgeState::Unavailable => Some(LLE_UNAVAILABLE),
            BridgeState::Active => Some("LLE live I/O bridge active"),
        }
    }

    /// Hands the engine ends to an engine thread (once).
    pub fn take_engine_ends(&mut self) -> Option<StampedEnds> {
        self.ends.take()
    }

    /// Pumps `backend` from the audio callback: [`Self::end_block`] drains events into it and
    /// publishes its telemetry. Returns `false` (and drops `backend`) unless the bridge is active
    /// and its ends are still here.
    pub fn attach_backend(&mut self, backend: Box<dyn LleBackendIo>) -> bool {
        let Some(ends) = self.ends.take() else {
            return false;
        };
        self.pump = Some(Pump {
            ends,
            backend,
            scratch: Box::default(),
        });
        true
    }

    /// Starts watching every host parameter: later changes are forwarded as panel events. The
    /// current values are the baseline, so nothing is sent for them. Allocates; call it from
    /// `initialize`. No-op unless active.
    pub fn bind_params(&mut self, params: &Stage4Params) {
        if !self.is_active() {
            return;
        }
        self.watched = params.param_map().into_iter().map(|(_, p, _)| p).collect();
        // SAFETY: the pointers belong to `params`, which the plugin keeps alive; read-only.
        self.shadow = self
            .watched
            .iter()
            .map(|p| unsafe { p.unmodulated_normalized_value() })
            .collect();
    }

    /// Forwards one host event at its block offset. Drops what the engine cannot take (polyphonic
    /// aftertouch, wildcard keys, system messages). Real-time safe.
    pub fn forward_host_event(&mut self, event: &NoteEvent<()>) {
        let Some(tx) = self.tx.as_mut() else { return };
        if let NoteEvent::NoteOn { note, .. }
        | NoteEvent::NoteOff { note, .. }
        | NoteEvent::Choke { note, .. } = event
        {
            if *note >= 128 {
                return;
            }
        }
        if let Some(core) = to_core_midi_event(event) {
            // A full ring is counted by the producer: `overflow_count`.
            tx.push(Stamped {
                offset: event.timing(),
                event: LleEvent::from(core),
            });
        }
    }

    /// Forwards every parameter that moved since the last call as a panel event at offset 0.
    /// Real-time safe.
    pub fn scan_params(&mut self) {
        let Some(tx) = self.tx.as_mut() else { return };
        for (i, (ptr, old)) in self.watched.iter().zip(self.shadow.iter_mut()).enumerate() {
            // SAFETY: see `bind_params`.
            let now = unsafe { ptr.unmodulated_normalized_value() };
            if now != *old {
                *old = now;
                if let Some(event) = panel_edit(PanelKind::Knob, i as u16, now) {
                    tx.push(Stamped { offset: 0, event });
                }
            }
        }
    }

    /// End of a block: with a backend attached, drains the ring into it and publishes telemetry.
    pub fn end_block(&mut self) {
        if let Some(p) = self.pump.as_mut() {
            drain_stamped(&mut *p.backend, &mut p.ends.events);
            p.backend
                .publish_telemetry(&mut p.scratch, &mut p.ends.telemetry);
        }
    }

    /// Events the full ring rejected since the bridge was created (0 when off).
    pub fn overflow_count(&self) -> u32 {
        self.tx.as_ref().map_or(0, Producer::dropped)
    }

    /// Latest telemetry into `out`, lock-free. `false` when the bridge is off or the read raced a
    /// publish (keep the previous frame and retry next frame).
    pub fn poll_telemetry(&self, out: &mut TelemetrySnapshot) -> bool {
        self.reader.as_ref().is_some_and(|r| r.read(out))
    }

    /// A clone of the telemetry reader for the editor thread.
    pub fn telemetry_reader(&self) -> Option<TelemetryReader> {
        self.reader.clone()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn default_bridge_is_off_and_silent() {
        let mut b = LleBridge::default();
        assert_eq!(b.state(), BridgeState::Off);
        assert!(!b.is_active());
        assert_eq!(b.status_text(), None);
        assert_eq!(b.overflow_count(), 0);
        b.scan_params();
        b.end_block();
        assert!(b.telemetry_reader().is_none());
    }

    #[test]
    fn unavailable_state_text_is_the_documented_one() {
        let b = LleBridge::from_settings_with(Some("lle"), None, || {
            LleAvailability::Unavailable("x".into())
        });
        assert_eq!(
            b.status_text(),
            Some("LLE engine not available in this build")
        );
    }
}
