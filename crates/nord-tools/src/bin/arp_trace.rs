//! Note events of the native arpeggiator for a program (`.ns4p`): which note starts / stops at
//! which time for held keys, per synth layer. Prints one JSON document (derived numbers only, no
//! audio) that `tools/arp_fw_compare.py` sets against the ARM firmware capture
//! (`docs/research/arp-fw-vs-native.md`).
//!
//! Usage: `cargo run --release -p nord-tools --bin arp_trace -- <program.ns4p> [--keys 60,64,67]
//! [--seconds 3] [--velocity 100] [--tempo 100] [--legacy-arp] [--arp-fw-gate] [--hold] [firmware option flags]`.
//! `--tempo` overrides the program's Master Clock tempo; `--hold` keeps the keys down for the
//! whole run (the default), `--release-at S` sends the note-offs at second `S`.

use nord_core::cbin::CbinFile;
use nord_core::engine::Stage4Engine;
use nord_core::midi::MidiEvent;
use nord_core::render_opts::FirmwareOptions;
use nord_core::synth::voice_edges::{VoiceEdge, VoiceEdgeTracker};
use nord_core::synth::SynthLayer;
use serde_json::{json, Value};

const FS: f32 = 48_000.0;

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// `"60,64,67"` to key numbers.
fn parse_keys(text: &str) -> Result<Vec<u8>, String> {
    text.split(',')
        .map(|k| k.trim().parse::<u8>().map_err(|_| format!("bad key {k:?}")))
        .collect()
}

/// The last path component (keeps the user's directories out of the committed JSON).
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Time of a frame in milliseconds.
fn ms(frame: u64) -> f64 {
    frame as f64 * 1000.0 / f64::from(FS)
}

fn edge_json(e: &VoiceEdge) -> Value {
    json!({"ms": ms(e.frame), "on": e.on, "note": e.note, "velocity": e.velocity})
}

fn layer_json(name: &str, layer: &SynthLayer, edges: &[VoiceEdge]) -> Value {
    json!({
        "layer": name,
        "arp_mode": format!("{:?}", layer.arp_mode),
        "clock_sync": layer.perf.arp_clock_sync,
        "division": format!("{:?}", layer.perf.arp_division),
        "arp_rate_bpm": layer.arp_rate_bpm,
        "master_bpm": layer.perf.master_bpm,
        "octave_range": layer.arp_octave_range,
        "range_extra_thirds": layer.perf.range_extra_thirds,
        "zigzag": layer.perf.zigzag,
        "pattern_on": layer.perf.pattern_on,
        "kb_sync": layer.perf.kb_sync,
        "step_ms": f64::from(layer.arp_step_seconds()) * 1000.0,
        "events": edges.iter().map(edge_json).collect::<Vec<_>>(),
    })
}

fn run(args: &[String]) -> Result<(), String> {
    let path = args
        .get(1)
        .ok_or("usage: arp_trace <program.ns4p> [--keys 60,64]")?;
    let keys = parse_keys(flag(args, "--keys").unwrap_or("60"))?;
    let num = |name: &str, d: f64| {
        flag(args, name).map_or(Ok(d), |v| v.parse().map_err(|_| format!("bad {name}")))
    };
    let seconds = num("--seconds", 3.0)?;
    let velocity = num("--velocity", 100.0)? as u8;
    let release_at =
        flag(args, "--release-at").map(|v| v.parse::<f64>().map_err(|_| "bad --release-at"));
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let prog = CbinFile::parse(&bytes)
        .and_then(|c| c.decode_program())
        .map_err(|e| format!("{path}: {e}"))?;
    let opts = FirmwareOptions::from_args(args)?;
    let mut engine = Stage4Engine::new(FS);
    engine.apply_program(&prog);
    opts.apply_to(&mut engine);
    if let Some(t) = flag(args, "--tempo") {
        engine.master_clock.bpm = t.parse().map_err(|_| "bad --tempo")?;
    }
    for &note in &keys {
        engine.handle_midi(MidiEvent::NoteOn {
            channel: 0,
            note,
            velocity,
        });
    }
    let frames = (seconds * f64::from(FS)) as u64;
    let release_frame = match release_at {
        Some(r) => Some((r.map_err(str::to_string)? * f64::from(FS)) as u64),
        None => None,
    };
    let mut trackers = [
        VoiceEdgeTracker::new(),
        VoiceEdgeTracker::new(),
        VoiceEdgeTracker::new(),
    ];
    let mut edges: [Vec<VoiceEdge>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for frame in 0..frames {
        if release_frame == Some(frame) {
            for &note in &keys {
                engine.handle_midi(MidiEvent::NoteOff {
                    channel: 0,
                    note,
                    velocity: 0,
                });
            }
        }
        engine.render_frame();
        trackers[0].step(&engine.synth_a, frame, &mut edges[0]);
        trackers[1].step(&engine.synth_b, frame, &mut edges[1]);
        trackers[2].step(&engine.synth_c, frame, &mut edges[2]);
    }
    let layers = [
        layer_json("A", &engine.synth_a, &edges[0]),
        layer_json("B", &engine.synth_b, &edges[1]),
        layer_json("C", &engine.synth_c, &edges[2]),
    ];
    println!(
        "{}",
        json!({"program": file_name(path), "arp_firmware": opts.arp_firmware, "arp_firmware_gate": opts.arp_firmware_gate, "keys": keys, "seconds": seconds, "layers": layers})
    );
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Err(e) = run(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_and_reject_garbage() {
        assert_eq!(parse_keys("60, 64,67").unwrap(), vec![60, 64, 67]);
        assert!(parse_keys("60,x").is_err());
        assert!(parse_keys("300").is_err());
    }

    #[test]
    fn file_name_drops_the_directories() {
        assert_eq!(file_name(r"C:\a\b/Horizon Whl.ns4p"), "Horizon Whl.ns4p");
        assert_eq!(file_name("x.ns4p"), "x.ns4p");
    }

    #[test]
    fn frames_convert_to_milliseconds() {
        assert!((ms(48_000) - 1000.0).abs() < 1e-9);
        assert!((ms(7_200) - 150.0).abs() < 1e-9);
    }

    #[test]
    fn edge_json_carries_time_note_and_velocity() {
        let e = VoiceEdge {
            frame: 4_800,
            on: true,
            note: 62,
            velocity: 100,
        };
        let v = edge_json(&e);
        assert_eq!(v["note"], 62);
        assert_eq!(v["on"], true);
        assert!((v["ms"].as_f64().unwrap() - 100.0).abs() < 1e-9);
    }
}
