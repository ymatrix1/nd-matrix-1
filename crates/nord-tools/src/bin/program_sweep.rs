//! Renders every `.ns4p` program of a Nord Stage 4 backup (`.ns4b`) twice (engine defaults and
//! `--firmware-all --firmware-routing`) and prints one JSON line per program with the decoded
//! program, the resolved sounds, the routing facts and the output-health statistics.
//!
//! Usage: `cargo run --release -p nord-tools --bin program_sweep -- <backup.ns4b>
//! [--frames N] [--limit N] [--only <substring>] [--every K] [--skip N] [--notes a,b,..] [--velocity V]
//! [--synth-velocity] [--legacy-synth-velocity] [--legacy-arp]
//! [--arp-fw-gate] [--legacy-fx-ports] [--legacy-fx-rate] [--legacy-morph] [--piano-touch] [--legacy-piano-touch] [--wheel V] [--aftertouch V]
//! [--pedal V]`
//! (`--synth-velocity` is a no-op since D142 made the firmware synth velocity law the default;
//! `--legacy-synth-velocity` renders both modes with the linear blend; `--legacy-arp` and
//! `--legacy-morph` restore the pre-D166 arpeggiator and morph law in both modes; `--piano-touch` /
//! `--legacy-piano-touch` force the firmware Touch curves on / off in both modes; `--wheel`,
//! `--aftertouch` and `--pedal` send that controller value (0..127) before the notes, so the
//! morph law is exercised);
//! `tools/program_sweep.py` drives it. Output
//! holds derived numbers only (no audio, no firmware or sample content). Release build only: a
//! debug build overflows the stack in the FX chains.

use nord_core::cbin::{CbinFile, Ns4BackupArchive};
use nord_core::engine::Stage4Engine;
use nord_core::render::render_timeline;
use nord_core::render_opts::FirmwareOptions;
use nord_core::sinc::key_rule::kernel_for_key;
use nord_core::sweep::{analyze, rms_delta_db};
use nord_core::synth::SynthSampleSound;
use nord_tools::sweep_flags::{apply_legacy_flags, controller_events};
use nord_tools::sweep_load::{
    build_engine, events, flag, load_sounds, panic_message, piano_of, synth_of, PianoCache,
    SynthCache, FS,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Standard note set: C3, G3, E4, C5 (spans the Low / Mid / High split regions).
const NOTES: [u8; 4] = [48, 55, 64, 72];
const VELOCITY: u8 = 100;

/// Kernel indices (`sinc::key_rule`) reachable over the key spans of a multisample's zones.
fn sample_kernels(s: &SynthSampleSound) -> Vec<usize> {
    let mut seen = [false; 6];
    for z in &s.zones {
        for key in z.key_low..=z.key_high {
            seen[kernel_for_key(key)] = true;
        }
    }
    (0..6).filter(|&k| seen[k]).collect()
}

/// `--synth-velocity` forces the firmware law on (the default since D142), `--legacy-synth-velocity`
/// forces the linear blend; the legacy flag wins.
fn apply_synth_velocity_flags(o: &mut FirmwareOptions, args: &[String]) {
    if args.iter().any(|a| a == "--synth-velocity") {
        o.synth_velocity_fw = true;
    }
    if args.iter().any(|a| a == "--legacy-synth-velocity") {
        o.synth_velocity_fw = false;
    }
}

/// `--piano-touch` forces the firmware Touch curves on, `--legacy-piano-touch` forces the
/// Medium curve with the ASSUMED power curves for the other sounds (the pre-flip default); the
/// legacy flag wins. Lets one build print the sweep before and after the Touch default flip.
fn apply_piano_touch_flags(o: &mut FirmwareOptions, args: &[String]) {
    if args.iter().any(|a| a == "--piano-touch") {
        o.piano_touch = true;
    }
    if args.iter().any(|a| a == "--legacy-piano-touch") {
        o.piano_touch = false;
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .ok_or("usage: program_sweep <backup.ns4b> [--frames N] [--limit N] [--only S] [--every K] [--notes a,b] [--velocity V]")?;
    let frames: usize = flag(&args, "--frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or(96_000);
    let limit: usize = flag(&args, "--limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let only = flag(&args, "--only");
    let hold = frames * 3 / 4;
    let notes: Vec<u8> = flag(&args, "--notes").map_or_else(
        || NOTES.to_vec(),
        |v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect(),
    );
    let velocity: u8 = flag(&args, "--velocity")
        .and_then(|v| v.parse().ok())
        .unwrap_or(VELOCITY)
        .min(127);
    let every: usize = flag(&args, "--every")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    // `--skip N --every K` shards a run over K processes (programs N, N+K, ...).
    let skip: usize = flag(&args, "--skip")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let archive = Ns4BackupArchive::open(path).map_err(|e| e.to_string())?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut pianos: PianoCache = HashMap::new();
    let mut synths: SynthCache = HashMap::new();
    let names: Vec<String> = archive
        .entries_with_ext(".ns4p")
        .iter()
        .map(|e| e.name.clone())
        .filter(|n| only.is_none_or(|s| n.contains(s.as_str())))
        .skip(skip)
        .step_by(every)
        .take(limit)
        .collect();
    std::panic::set_hook(Box::new(|_| {}));
    let mut modes = [
        ("default", FirmwareOptions::default()),
        (
            "firmware",
            FirmwareOptions {
                firmware_routing: true,
                ..FirmwareOptions::firmware_all()
            },
        ),
    ];
    for (_, o) in &mut modes {
        apply_synth_velocity_flags(o, &args);
        apply_piano_touch_flags(o, &args);
        apply_legacy_flags(o, &args);
    }
    let byte = |name: &str| flag(&args, name).and_then(|v| v.parse::<u8>().ok());
    let controls = controller_events(byte("--wheel"), byte("--aftertouch"), byte("--pedal"));
    if args.iter().any(|a| a == "--legacy-1khz-tick") {
        // the pre-D131 1 ms ARM tick, to compare a sweep before and after the flip
        for (_, o) in &mut modes {
            o.measured_tick = false;
        }
    }
    for name in names {
        let bytes = archive
            .read_entry(&mut file, &name)
            .map_err(|e| e.to_string())?;
        let prog = CbinFile::parse(&bytes)
            .and_then(|c| c.decode_program())
            .map_err(|e| format!("{name}: {e}"))?;
        let refs = archive.resolve_program_sounds(&prog);
        load_sounds(&archive, &mut file, &refs, &mut pianos, &mut synths);
        let mut runs = serde_json::Map::new();
        for (label, opts) in &modes {
            let result = catch_unwind(AssertUnwindSafe(|| {
                let mut engine = build_engine(&prog, &refs, &pianos, &synths, opts);
                let mut timeline = controls.clone();
                timeline.extend(events(&notes, velocity, hold));
                let (l, r) = render_timeline(&mut engine, &timeline, frames);
                analyze(&l, &r)
            }));
            let value = match result {
                Ok(s) => serde_json::to_value(s).map_err(|e| e.to_string())?,
                Err(p) => json!({ "panic": panic_message(p) }),
            };
            runs.insert((*label).to_string(), value);
        }
        let rms = |k: &str| runs[k]["rms"].as_f64().map(|x| x as f32);
        let delta = rms("default")
            .zip(rms("firmware"))
            .and_then(|(a, b)| rms_delta_db(a, b));
        let routing = {
            let mut e = Stage4Engine::new(FS);
            e.apply_program(&prog);
            let r = e.program_routing();
            json!({ "exact_masks": r.exact_masks, "legacy_masks": r.legacy_masks,
                    "kb_zone": r.kb_zone, "split_active": r.split_active })
        };
        let class =
            |r: &Option<String>| piano_of(&pianos, r).map_or(Value::Null, |(_, c)| json!(c));
        let sound = |r: &Option<String>, ok: bool| json!({ "ref": r.is_some(), "loaded": ok });
        // SAMPLES layers: kernels the firmware key rule reaches over the key span of the zones.
        let kernels = |r: &Option<String>| {
            synth_of(&synths, r).map_or(Value::Null, |s| json!(sample_kernels(s)))
        };
        let line = json!({
            "entry": name,
            "program": serde_json::from_str::<Value>(&prog.to_json().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
            "sounds": {
                "piano_a": sound(&refs.piano_a, piano_of(&pianos, &refs.piano_a).is_some()),
                "piano_b": sound(&refs.piano_b, piano_of(&pianos, &refs.piano_b).is_some()),
                "synth_a": sound(&refs.synth_a, synth_of(&synths, &refs.synth_a).is_some()),
                "synth_a_kernels": kernels(&refs.synth_a),
                "synth_b": sound(&refs.synth_b, synth_of(&synths, &refs.synth_b).is_some()),
                "synth_b_kernels": kernels(&refs.synth_b),
                "synth_c": sound(&refs.synth_c, synth_of(&synths, &refs.synth_c).is_some()),
                "synth_c_kernels": kernels(&refs.synth_c),
                "piano_a_class": class(&refs.piano_a),
                "piano_b_class": class(&refs.piano_b),
            },
            "routing": routing,
            "renders": runs,
            "rms_delta_db": delta,
        });
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn synth_velocity_flags_default_on_and_legacy_wins() {
        let mut o = FirmwareOptions::default();
        apply_synth_velocity_flags(&mut o, &args(&[]));
        assert!(o.synth_velocity_fw);
        apply_synth_velocity_flags(&mut o, &args(&["--legacy-synth-velocity"]));
        assert!(!o.synth_velocity_fw);
        apply_synth_velocity_flags(&mut o, &args(&["--synth-velocity"]));
        assert!(o.synth_velocity_fw);
        apply_synth_velocity_flags(
            &mut o,
            &args(&["--synth-velocity", "--legacy-synth-velocity"]),
        );
        assert!(!o.synth_velocity_fw);
    }

    #[test]
    fn piano_touch_flags_force_either_way_and_legacy_wins() {
        let mut o = FirmwareOptions::default();
        apply_piano_touch_flags(&mut o, &args(&["--legacy-piano-touch"]));
        assert!(!o.piano_touch);
        apply_piano_touch_flags(&mut o, &args(&["--piano-touch"]));
        assert!(o.piano_touch);
        apply_piano_touch_flags(&mut o, &args(&["--piano-touch", "--legacy-piano-touch"]));
        assert!(!o.piano_touch);
    }

    #[test]
    fn sample_kernels_lists_ascending_reachable_kernel_indices() {
        let pcm: Vec<f32> = (0..256).map(|i| (i as f32 * 0.1).sin()).collect();
        let s = SynthSampleSound::new_synthetic("T", 60, &pcm, FS, None, 32);
        let k = sample_kernels(&s);
        assert!(!k.is_empty());
        assert!(k.windows(2).all(|w| w[0] < w[1]));
        assert!(k.iter().all(|&i| i < 6));
    }
}
