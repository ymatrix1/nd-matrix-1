//! Read-only view of the voice gates of a [`SynthLayer`]: note-on / note-off edges per frame.
//!
//! Evaluation helper for `crates/nord-tools` `arp_trace` (arpeggiator note events of the native
//! engine against the ARM firmware, `docs/research/arp-fw-vs-native.md`). It never changes the
//! layer, so no render path moves.

use super::SynthLayer;

/// One gate edge of a voice slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceEdge {
    /// Output frame at which the edge was seen (the frame just rendered).
    pub frame: u64,
    /// `true` = gate opened (note on, also a retrigger of the slot), `false` = gate closed.
    pub on: bool,
    pub note: u8,
    /// Voice velocity `0..=127` (rounded from the voice's normalized value).
    pub velocity: u8,
}

impl SynthLayer {
    /// `(slot, note, gate, velocity 0..=1)` of every active voice, in slot order.
    pub fn active_voices(&self) -> impl Iterator<Item = (usize, u8, bool, f32)> + '_ {
        self.voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.active)
            .map(|(i, v)| (i, v.note, v.gate, v.velocity))
    }

    /// Arp / Gate step length in seconds the layer currently plays (Master Clock or Rate dial).
    pub fn arp_step_seconds(&self) -> f32 {
        self.step_seconds()
    }
}

/// Tracks the gate of every voice slot and reports the edges between two calls.
#[derive(Debug, Clone, Default)]
pub struct VoiceEdgeTracker {
    prev: Vec<(bool, u8)>,
}

impl VoiceEdgeTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compares the layer with the state at the previous call and appends the edges to `out`.
    /// A slot whose note changed while its gate stayed open counts as off + on.
    pub fn step(&mut self, layer: &SynthLayer, frame: u64, out: &mut Vec<VoiceEdge>) {
        let mut now: Vec<(bool, u8)> = vec![(false, 0); self.prev.len().max(16)];
        if self.prev.len() < now.len() {
            self.prev.resize(now.len(), (false, 0));
        }
        let mut vel = vec![0u8; now.len()];
        for (slot, note, gate, v) in layer.active_voices() {
            if slot < now.len() {
                now[slot] = (gate, note);
                vel[slot] = (v * 127.0).round().clamp(0.0, 127.0) as u8;
            }
        }
        for (slot, &(gate, note)) in now.iter().enumerate() {
            let (pgate, pnote) = self.prev[slot];
            if pgate && (!gate || note != pnote) {
                out.push(VoiceEdge {
                    frame,
                    on: false,
                    note: pnote,
                    velocity: 0,
                });
            }
            if gate && (!pgate || note != pnote) {
                out.push(VoiceEdge {
                    frame,
                    on: true,
                    note,
                    velocity: vel[slot],
                });
            }
        }
        self.prev = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_follow_note_on_and_off() {
        let mut layer = SynthLayer::new(48_000.0);
        let mut tracker = VoiceEdgeTracker::new();
        let mut edges = Vec::new();
        tracker.step(&layer, 0, &mut edges);
        assert!(edges.is_empty());
        layer.note_on(60, 100);
        tracker.step(&layer, 1, &mut edges);
        assert_eq!(edges.len(), 1);
        assert!(edges[0].on && edges[0].note == 60 && edges[0].frame == 1);
        assert_eq!(edges[0].velocity, 100);
        tracker.step(&layer, 2, &mut edges);
        assert_eq!(edges.len(), 1, "no edge while the gate stays open");
        layer.note_off(60);
        tracker.step(&layer, 3, &mut edges);
        assert_eq!(edges.len(), 2);
        assert!(!edges[1].on && edges[1].note == 60 && edges[1].frame == 3);
    }

    #[test]
    fn arp_step_seconds_is_an_eighth_at_the_rate_dial_and_follows_the_clock_when_synced() {
        let mut layer = SynthLayer::new(48_000.0);
        layer.arp_rate_bpm = 120.0;
        assert!((layer.arp_step_seconds() - 0.25).abs() < 1e-6);
        layer.perf.arp_clock_sync = true;
        layer.perf.master_bpm = 100.0;
        layer.perf.arp_division = crate::synth::ClockDivision::Sixteenth;
        assert!((layer.arp_step_seconds() - 0.15).abs() < 1e-6);
    }

    #[test]
    fn active_voices_lists_only_sounding_slots() {
        let mut layer = SynthLayer::new(48_000.0);
        assert_eq!(layer.active_voices().count(), 0);
        layer.note_on(64, 90);
        let voices: Vec<_> = layer.active_voices().collect();
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].1, 64);
        assert!(voices[0].2);
    }
}
