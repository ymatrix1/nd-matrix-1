//! Bench workloads with a firmware organ stage running (TD-15, bean `ns4-4wzo`).
//!
//! The B3 Bass (D71) and Farfisa (D74) stages are opt-in and bit exact against the firmware, so
//! their cost is a fixed number of SHARC instructions per DSP call whatever the keys. These
//! engines are the `typical` workload (Scene I, 10 keys held, default FX) with Organ A switched to
//! the model and the stage on: `ns4 bench --scenario organ-bass` / `organ-farfisa` measure the
//! block cost against the 0.8 ms budget of the PRD.

use crate::engine::Stage4Engine;
use crate::organ::{OrganModel, ToneWheelSource};

/// The typical engine with Organ A on the Bass (`bass`) or Farfisa stage.
pub fn organ_fw_engine(sample_rate: f32, bass: bool) -> Stage4Engine {
    let mut e = Stage4Engine::new(sample_rate);
    let o = &mut e.organ_a;
    if bass {
        o.model = OrganModel::B3;
        o.b3_bass_mode = true;
        o.tonewheel_source = ToneWheelSource::B3Wheels;
        o.set_b3_bass_processor(true);
    } else {
        o.model = OrganModel::Farfisa;
        o.tonewheel_source = ToneWheelSource::FarfisaWheels;
        o.set_farfisa_processor(true);
    }
    // the same 10 note chord as `typical_engine`
    for note in 55..65 {
        e.handle_midi(crate::midi::MidiEvent::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        });
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{run_engine, typical_engine, BenchConfig, Scenario};

    fn cfg() -> BenchConfig {
        BenchConfig {
            scenario: Scenario::OrganBass,
            warmup_blocks: 20,
            blocks: 40,
            multi_out: true,
        }
    }

    #[test]
    fn the_stage_runs_and_changes_the_output_of_the_typical_workload() {
        let typical = run_engine(cfg(), typical_engine(48_000.0));
        for bass in [true, false] {
            if !bass && !matches!(crate::organ::farfisa_firmware_status(), Ok(true)) {
                // the Farfisa stage needs the user's OS update file (`NS4_WAVE_TABLES`, D128)
                eprintln!("no OS update file: skipping the Farfisa stage workload");
                continue;
            }
            let r = run_engine(cfg(), organ_fw_engine(48_000.0, bass));
            assert!(r.all_finite && r.peak > 0.0);
            assert_ne!(r.hash, typical.hash, "bass {bass}");
        }
    }
}
