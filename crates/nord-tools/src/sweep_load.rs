//! Program loading and note scheduling shared by the factory-program sweep binaries
//! (`program_sweep`, `program_rank`): sound caches, engine construction for one decoded program,
//! the held-note event list and small argument helpers. Offline tooling only.

use nord_core::cbin::{Ns4BackupArchive, ProgramDecoded, ProgramSoundRefs};
use nord_core::engine::Stage4Engine;
use nord_core::midi::MidiEvent;
use nord_core::piano::NpnoSound;
use nord_core::render::TimedEvent;
use nord_core::render_opts::FirmwareOptions;
use nord_core::synth::SynthSampleSound;
use std::collections::HashMap;
use std::fs::File;
use std::sync::Arc;

/// Render sample rate of the sweeps.
/// ASSUMED: the engine's internal rate (`Stage4Engine::new(FS)`), not a hardware value; the
/// hardware audio rate is not used by any ranking number.
pub const FS: f32 = 48_000.0;

/// Loaded `.npno` sound and its class byte (file offset 0x44), `None` when it failed to load.
pub type PianoCache = HashMap<String, Option<(Arc<NpnoSound>, u8)>>;
/// Loaded SAMPLES multisample, `None` when it failed to load.
pub type SynthCache = HashMap<String, Option<Arc<SynthSampleSound>>>;

/// Value after the flag `name` in `args`.
pub fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
}

/// Note-on at frame 0 and note-off at frame `hold` for every note.
pub fn events(notes: &[u8], velocity: u8, hold: usize) -> Vec<TimedEvent> {
    let mut v = Vec::new();
    for &note in notes {
        v.push(TimedEvent {
            frame: 0,
            event: MidiEvent::NoteOn {
                channel: 0,
                note,
                velocity,
            },
        });
        v.push(TimedEvent {
            frame: hold,
            event: MidiEvent::NoteOff {
                channel: 0,
                note,
                velocity: 0,
            },
        });
    }
    v
}

/// Cached `.npno` sound of the reference `r`.
pub fn piano_of<'a>(c: &'a PianoCache, r: &Option<String>) -> Option<&'a (Arc<NpnoSound>, u8)> {
    r.as_ref().and_then(|r| c.get(r)).and_then(Option::as_ref)
}

/// Cached SAMPLES multisample of the reference `r`.
pub fn synth_of<'a>(c: &'a SynthCache, r: &Option<String>) -> Option<&'a Arc<SynthSampleSound>> {
    r.as_ref().and_then(|r| c.get(r)).and_then(Option::as_ref)
}

/// Reads every sound `refs` names that is not cached yet from the backup.
pub fn load_sounds(
    archive: &Ns4BackupArchive,
    file: &mut File,
    refs: &ProgramSoundRefs,
    pianos: &mut PianoCache,
    synths: &mut SynthCache,
) {
    for r in [&refs.piano_a, &refs.piano_b].into_iter().flatten() {
        if !pianos.contains_key(r) {
            let loaded = archive.read_entry(file, r).ok().and_then(|b| {
                let class = b.get(0x44).copied().unwrap_or(0);
                NpnoSound::from_npno_bytes(&b)
                    .ok()
                    .map(|s| (Arc::new(s), class))
            });
            pianos.insert(r.clone(), loaded);
        }
    }
    for r in [&refs.synth_a, &refs.synth_b, &refs.synth_c]
        .into_iter()
        .flatten()
    {
        if !synths.contains_key(r) {
            let loaded = archive
                .read_entry(file, r)
                .ok()
                .and_then(|b| SynthSampleSound::from_nsmp4_bytes(&b).ok().map(Arc::new));
            synths.insert(r.clone(), loaded);
        }
    }
}

/// Builds an engine for `prog` with the cached sounds installed (mirrors
/// `load_program_bytes_with_sounds_from_backup`) and `opts` applied.
pub fn build_engine(
    prog: &ProgramDecoded,
    refs: &ProgramSoundRefs,
    pianos: &PianoCache,
    synths: &SynthCache,
    opts: &FirmwareOptions,
) -> Stage4Engine {
    let mask = prog.global.scene_1_mask | prog.global.scene_2_mask;
    let needed = |on: bool, bit: u8| on || mask == 0 || mask & (1 << bit) != 0;
    let mut engine = Stage4Engine::new(FS);
    engine.apply_program(prog);
    if needed(prog.piano.layer_a.enabled, 2) {
        if let Some((s, _)) = piano_of(pianos, &refs.piano_a) {
            engine.piano_a.set_sound(Some(Arc::clone(s)));
        }
    }
    if needed(prog.piano.layer_b.enabled, 3) {
        if let Some((s, _)) = piano_of(pianos, &refs.piano_b) {
            engine.piano_b.set_sound(Some(Arc::clone(s)));
        }
    }
    if needed(prog.synth.layer_a.enabled, 4) {
        if let Some(s) = synth_of(synths, &refs.synth_a) {
            engine.synth_a.load_sample(Some(Arc::clone(s)));
        }
    }
    if needed(prog.synth.layer_b.enabled, 5) {
        if let Some(s) = synth_of(synths, &refs.synth_b) {
            engine.synth_b.load_sample(Some(Arc::clone(s)));
        }
    }
    if needed(prog.synth.layer_c.enabled, 6) {
        if let Some(s) = synth_of(synths, &refs.synth_c) {
            engine.synth_c.load_sample(Some(Arc::clone(s)));
        }
    }
    opts.apply_to(&mut engine);
    engine
}

/// Text of a caught panic payload (`String` or `&str`), empty otherwise.
pub fn panic_message(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flag_returns_the_value_after_the_name() {
        let a = args(&["x.ns4b", "--frames", "100", "--limit"]);
        assert_eq!(flag(&a, "--frames").map(String::as_str), Some("100"));
        assert_eq!(flag(&a, "--limit"), None, "no value after the last flag");
        assert_eq!(flag(&a, "--only"), None);
    }

    #[test]
    fn events_pair_each_note_on_at_zero_with_a_note_off_at_hold() {
        let ev = events(&[48, 60], 90, 1000);
        assert_eq!(ev.len(), 4);
        assert!(ev.iter().step_by(2).all(|e| e.frame == 0));
        assert!(ev.iter().skip(1).step_by(2).all(|e| e.frame == 1000));
        assert!(matches!(
            ev[0].event,
            MidiEvent::NoteOn {
                note: 48,
                velocity: 90,
                ..
            }
        ));
    }

    #[test]
    fn panic_message_reads_string_and_str_payloads() {
        assert_eq!(panic_message(Box::new(String::from("a"))), "a");
        assert_eq!(panic_message(Box::new("b")), "b");
        assert_eq!(panic_message(Box::new(3u8)), "");
    }

    #[test]
    fn caches_resolve_none_for_missing_references() {
        let (p, s): (PianoCache, SynthCache) = (HashMap::new(), HashMap::new());
        assert!(piano_of(&p, &None).is_none());
        assert!(piano_of(&p, &Some("x".into())).is_none());
        assert!(synth_of(&s, &Some("x".into())).is_none());
    }

    #[test]
    fn build_engine_on_the_default_program_renders_finite() {
        let prog = ProgramDecoded::default();
        let refs = ProgramSoundRefs::default();
        let mut e = build_engine(
            &prog,
            &refs,
            &HashMap::new(),
            &HashMap::new(),
            &FirmwareOptions::default(),
        );
        let (l, r) = nord_core::render::render_timeline(&mut e, &events(&[60], 100, 200), 400);
        assert!(l.iter().chain(&r).all(|x| x.is_finite()));
    }
}
