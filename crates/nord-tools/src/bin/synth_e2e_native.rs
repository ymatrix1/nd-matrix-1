//! Native counterpart of `tools/synth_e2e_oracle.py`: renders ONE synth layer of a program
//! (`.ns4p`) for a held note through `nord-core` and writes the layer output (before pan, layer
//! mix, FX and master) as raw little-endian `f32` stereo frames. Prints one JSON line of derived
//! numbers (no audio content).
//!
//! Usage: `cargo run --release -p nord-tools --bin synth_e2e_native -- <program.ns4p> --out <f32>
//! [--layer 0|1|2] [--note 60] [--velocity 100] [--frames 96000] [--start 0] [--hold 72000] [--no-filter]
//! [firmware option flags of `ns4 render-patch`, e.g. --synth-fw-filter]`.
//! Analog-oscillator programs only (no sample bank is loaded).

use nord_core::cbin::CbinFile;
use nord_core::engine::Stage4Engine;
use nord_core::render_opts::FirmwareOptions;
use nord_core::synth::SynthLayer;
use serde_json::json;

const FS: f32 = 48_000.0;

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// Frames `[left, right]` of `layer`: `note` on at frame `start`, off at frame `hold`.
fn render_layer(
    layer: &mut SynthLayer,
    note: u8,
    velocity: u8,
    frames: usize,
    start: usize,
    hold: usize,
) -> Vec<[f32; 2]> {
    (0..frames)
        .map(|n| {
            if n == start {
                layer.note_on(note, velocity);
            }
            if n == hold {
                layer.note_off(note);
            }
            layer.render_frame()
        })
        .collect()
}

fn run(args: &[String]) -> Result<(), String> {
    let path = args
        .get(1)
        .ok_or("usage: synth_e2e_native <program.ns4p> --out F")?;
    let out = flag(args, "--out").ok_or("--out FILE required")?;
    let num = |name: &str, d: usize| {
        flag(args, name).map_or(Ok(d), |v| v.parse().map_err(|_| format!("bad {name}")))
    };
    let (layer_ix, note, vel) = (
        num("--layer", 0)?,
        num("--note", 60)? as u8,
        num("--velocity", 100)? as u8,
    );
    let (frames, start, hold) = (
        num("--frames", 96_000)?,
        num("--start", 0)?,
        num("--hold", 72_000)?,
    );
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let prog = CbinFile::parse(&bytes)
        .and_then(|c| c.decode_program())
        .map_err(|e| format!("{path}: {e}"))?;
    let opts = FirmwareOptions::from_args(args)?;
    let mut engine = Stage4Engine::new(FS);
    engine.apply_program(&prog);
    opts.apply_to(&mut engine);
    let layer = match layer_ix {
        0 => &mut *engine.synth_a,
        1 => &mut *engine.synth_b,
        2 => &mut *engine.synth_c,
        _ => return Err("--layer must be 0, 1 or 2".into()),
    };
    if args.iter().any(|a| a == "--no-filter") {
        layer.filter_on = false;
    }
    let info = json!({
        "cutoff_hz": layer.cutoff_hz,
        "filter_on": layer.filter_on,
        "octave": layer.octave_offset,
        "unison": layer.unison,
        "amp_env": [layer.attack_sec, layer.decay_sec, layer.sustain_level, layer.release_sec],
        "filter_env": [
            layer.filter_env_attack_sec,
            layer.filter_env_decay_sec,
            layer.filter_env_release_sec,
            layer.filter_env_amount
        ],
    });
    let fr = render_layer(layer, note, vel, frames, start, hold);
    let raw: Vec<u8> = fr
        .iter()
        .flat_map(|f| f.iter().flat_map(|x| x.to_le_bytes()))
        .collect();
    std::fs::write(out, raw).map_err(|e| format!("{out}: {e}"))?;
    let peak = fr
        .iter()
        .flat_map(|f| f.iter())
        .fold(0f32, |m, x| m.max(x.abs()));
    println!(
        "{}",
        json!({"frames": fr.len(), "peak": peak, "layer": info})
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
    fn render_layer_gives_one_frame_per_step_and_a_held_note_is_audible() {
        let mut l = SynthLayer::new(FS);
        let fr = render_layer(&mut l, 60, 100, 4_800, 1_000, 3_000);
        assert_eq!(fr.len(), 4_800);
        assert!(fr.iter().flatten().all(|x| x.is_finite()));
        assert!(
            fr[..1_000].iter().flatten().all(|x| *x == 0.0),
            "silent before the note"
        );
        assert!(fr[1_000..3_000].iter().flatten().any(|x| x.abs() > 1e-4));
    }

    #[test]
    fn flag_reads_the_value_after_the_name() {
        let a: Vec<String> = ["x", "--note", "64"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(flag(&a, "--note"), Some("64"));
        assert_eq!(flag(&a, "--layer"), None);
    }
}
