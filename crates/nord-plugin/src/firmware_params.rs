//! Host switches for the firmware-law defaults of the engine (D169).
//!
//! The engine runs the firmware arpeggiator walk and morph law and the piano Touch curves by
//! default (D165, D166); the offline tools can opt out (`--legacy-arp`, `--legacy-morph`,
//! `--legacy-piano-touch`). The plugin had no such switch and took the engine defaults silently.
//! IDs are additive: `fwx_*`. Defaults equal the engine defaults, so an untouched host renders
//! bit-identically. These are engine behaviour flags, not program data, so the host value is always
//! written (no program override tracking, see `program_overrides.rs`).

use nih_plug::prelude::*;
use nord_core::engine::Stage4Engine;

/// Firmware-law switches.
///
/// ASSUMED: the four defaults mirror the engine defaults (`SynthLayer::new`, `Stage4Engine::new`,
/// `PianoLayer::new`: walk, gate, morph law and Touch curves on, D166 and D168); nothing here is a
/// measured value. `defaults_equal_the_engine_defaults` fails if an engine default moves.
#[derive(Params)]
pub struct FirmwareFlagParams {
    /// Arpeggiator UP / DOWN walk over the whole octave pool, a single key retriggers
    /// (`SynthLayer::arp_firmware_walk`, D162, D166). Off = legacy arp (`--legacy-arp`).
    #[id = "fwx_arp_walk"]
    pub arp_walk: BoolParam,
    /// 2/3 step gate of Arp / Poly notes (`SynthLayer::arp_firmware_gate`). Default on (D168);
    /// off = the engine gate (`--no-arp-gate`).
    #[id = "fwx_arp_gate"]
    pub arp_gate: BoolParam,
    /// Firmware morph law, `v / 128` weights (`Stage4Engine::firmware_morph`, D159, D166).
    /// Off = legacy morph (`--legacy-morph`).
    #[id = "fwx_morph_law"]
    pub morph_law: BoolParam,
    /// Firmware piano Touch curves (`PianoLayer::firmware_touch`, D165). Off = the Medium curve
    /// for every note (`--legacy-piano-touch`).
    #[id = "fwx_touch_curves"]
    pub touch_curves: BoolParam,
}

impl Default for FirmwareFlagParams {
    fn default() -> Self {
        Self {
            arp_walk: BoolParam::new("Firmware Arp Walk", true),
            arp_gate: BoolParam::new("Firmware Arp Gate (2/3)", true),
            morph_law: BoolParam::new("Firmware Morph Law", true),
            touch_curves: BoolParam::new("Firmware Piano Touch Curves", true),
        }
    }
}

impl FirmwareFlagParams {
    /// Plain field stores; audio-thread safe.
    pub fn sync_to_engine(&self, engine: &mut Stage4Engine) {
        let (walk, gate) = (self.arp_walk.value(), self.arp_gate.value());
        for s in [
            &mut engine.synth_a,
            &mut engine.synth_b,
            &mut engine.synth_c,
        ] {
            s.arp_firmware_walk = walk;
            s.arp_firmware_gate = gate;
        }
        engine.firmware_morph = self.morph_law.value();
        let curves = self.touch_curves.value();
        engine.piano_a.firmware_touch = curves;
        engine.piano_b.firmware_touch = curves;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_equal_the_engine_defaults() {
        let reference = Stage4Engine::new(48_000.0);
        let mut e = Stage4Engine::new(48_000.0);
        // Perturb first so the test fails if sync writes nothing.
        e.firmware_morph = false;
        e.synth_b.arp_firmware_walk = false;
        e.synth_c.arp_firmware_gate = false;
        e.piano_a.firmware_touch = false;
        FirmwareFlagParams::default().sync_to_engine(&mut e);
        assert_eq!(e.firmware_morph, reference.firmware_morph);
        assert_eq!(e.piano_a.firmware_touch, reference.piano_a.firmware_touch);
        assert_eq!(e.piano_b.firmware_touch, reference.piano_b.firmware_touch);
        for (a, b) in [
            (&e.synth_a, &reference.synth_a),
            (&e.synth_b, &reference.synth_b),
            (&e.synth_c, &reference.synth_c),
        ] {
            assert_eq!(a.arp_firmware_walk, b.arp_firmware_walk);
            assert_eq!(a.arp_firmware_gate, b.arp_firmware_gate);
        }
    }

    #[test]
    fn host_switches_reach_the_engine() {
        let p = FirmwareFlagParams {
            arp_walk: BoolParam::new("w", false),
            arp_gate: BoolParam::new("g", false),
            morph_law: BoolParam::new("m", false),
            touch_curves: BoolParam::new("t", false),
        };
        let mut e = Stage4Engine::new(48_000.0);
        p.sync_to_engine(&mut e);
        assert!(!e.firmware_morph);
        assert!(!e.piano_a.firmware_touch && !e.piano_b.firmware_touch);
        for s in [&e.synth_a, &e.synth_b, &e.synth_c] {
            assert!(!s.arp_firmware_walk && !s.arp_firmware_gate);
        }
    }

    #[test]
    fn ids_are_unique_and_prefixed() {
        let ids: Vec<String> = FirmwareFlagParams::default()
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(ids.len(), 4);
        assert!(ids.iter().all(|i| i.starts_with("fwx_")));
    }
}
