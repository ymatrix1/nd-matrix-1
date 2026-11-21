//! Native side of `tools/synth_env_trace.py` (D195): the strip envelope chains and the cutoff
//! the native engine derives from them, one value per firmware pass, for one synth layer of a
//! program (`.ns4p`).
//!
//! Usage: `cargo run --release -p nord-tools --bin synth_env_native -- <program.ns4p> [--layer 0|1|2]
//! [--on 300] [--off 11149] [--passes 16575]`. Prints one JSON line: the stored knobs, the
//! record words, the panel cutoff and envelope amount, and per pass the amp level, the filter
//! level, and the native cutoff coefficient `R` (strip word law of `fw_strip_cutoff.rs`) after
//! the engine's `2^(amount x octaves x level)` filter envelope law. No audio is produced.

use nord_core::cbin::{CbinFile, FILTER_CUTOFF_HZ_TABLE};
use nord_core::engine::Stage4Engine;
use nord_core::synth::{
    fw_strip_cutoff_r, strip_word_for_cutoff, StripEnvRecords, StripKnobs, StripParams, StripVoice,
    SynthLayer,
};
use serde_json::{json, Value};

const FS: f32 = 48_000.0;
/// Octaves of the native filter envelope at full amount (`FILTER_ENV_OCTAVES` in `synth/mod.rs`).
const NATIVE_ENV_OCTAVES: f32 = 8.0;

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// `(amp level, filter level)` after every pass: `on` passes of silence, the gate open until
/// `off`, then released. `StripParams::period_frames = 1` makes one `advance` one pass.
pub fn chain_levels(p: &StripParams, on: usize, off: usize, passes: usize) -> Vec<(f32, f32)> {
    let mut v = StripVoice::default();
    (0..passes)
        .map(|n| {
            if n < on {
                (0.0, 0.0)
            } else {
                v.advance(p, n < off)
            }
        })
        .collect()
}

/// Cutoff knob 0..=127 of a stored cutoff (nearest table entry, the decoder reads `table[knob]`).
pub fn cutoff_knob(hz: f32) -> usize {
    (0..FILTER_CUTOFF_HZ_TABLE.len())
        .min_by(|&a, &b| {
            let d = |i: usize| (FILTER_CUTOFF_HZ_TABLE[i] - hz).abs();
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(0)
}

/// Native cutoff coefficient for a filter envelope level (`2^(amount x octaves x level)`).
pub fn native_r(cutoff_hz: f32, amount: f32, level: f32) -> f32 {
    let scale = 2.0f32.powf(amount.clamp(-1.0, 1.0) * NATIVE_ENV_OCTAVES * level);
    fw_strip_cutoff_r(strip_word_for_cutoff(cutoff_hz * scale))
}

fn records_json(r: &StripEnvRecords) -> Value {
    json!({
        "attack": [r.attack.a, r.attack.b, r.attack.thr],
        "decay": [r.decay.a, r.decay.b, r.decay.thr],
        "release": [r.release.a, r.release.b, r.release.thr],
    })
}

fn knobs_json(k: &StripKnobs) -> Value {
    json!({
        "amp": [k.amp.attack, k.amp.decay, k.amp.release],
        "filter": [k.filter.attack, k.filter.decay, k.filter.release],
    })
}

fn layer_report(layer: &SynthLayer, on: usize, off: usize, passes: usize) -> Result<Value, String> {
    let knobs = layer
        .strip_knobs
        .ok_or("program carries no envelope knobs")?;
    let params = StripParams {
        amp: StripEnvRecords::from_knobs(knobs.amp, layer.sustain_level),
        filter: StripEnvRecords::from_knobs(knobs.filter, 0.0),
        period_frames: 1.0,
    };
    let lv = chain_levels(&params, on, off, passes);
    let amount = layer.filter_env_amount;
    let r: Vec<f32> = lv
        .iter()
        .map(|&(_, f)| native_r(layer.cutoff_hz, amount, f))
        .collect();
    Ok(json!({
        "knobs": knobs_json(&knobs),
        "amp_records": records_json(&params.amp),
        "filter_records": records_json(&params.filter),
        "cutoff_hz": layer.cutoff_hz,
        "cutoff_knob": cutoff_knob(layer.cutoff_hz),
        "filter_env_amount_knob": (amount * 127.0).round(),
        "kb_track": layer.kb_track,
        "resonance": layer.resonance,
        "filter_env_amount": amount,
        "filter_env_dedicated": layer.filter_env_dedicated,
        "sustain_level": layer.sustain_level,
        "amp": lv.iter().map(|l| l.0).collect::<Vec<_>>(),
        "filter": lv.iter().map(|l| l.1).collect::<Vec<_>>(),
        "r": r,
    }))
}

fn run(args: &[String]) -> Result<(), String> {
    let path = args
        .get(1)
        .ok_or("usage: synth_env_native <program.ns4p>")?;
    let num = |name: &str, d: usize| {
        flag(args, name).map_or(Ok(d), |v| v.parse().map_err(|_| format!("bad {name}")))
    };
    let (layer_ix, on, off, passes) = (
        num("--layer", 0)?,
        num("--on", 300)?,
        num("--off", 11_149)?,
        num("--passes", 16_575)?,
    );
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let prog = CbinFile::parse(&bytes)
        .and_then(|c| c.decode_program())
        .map_err(|e| format!("{path}: {e}"))?;
    let mut engine = Stage4Engine::new(FS);
    engine.apply_program(&prog);
    let layer = match layer_ix {
        0 => &*engine.synth_a,
        1 => &*engine.synth_b,
        2 => &*engine.synth_c,
        _ => return Err("--layer must be 0, 1 or 2".into()),
    };
    println!("{}", layer_report(layer, on, off, passes)?);
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
    use nord_core::synth::EnvKnobs;

    fn params() -> StripParams {
        let k = EnvKnobs {
            attack: 0,
            decay: 60,
            release: 50,
        };
        StripParams {
            amp: StripEnvRecords::from_knobs(k, 0.5),
            filter: StripEnvRecords::from_knobs(k, 0.0),
            period_frames: 1.0,
        }
    }

    #[test]
    fn levels_are_silent_before_the_key_rise_then_fall_after_it_goes_off() {
        let lv = chain_levels(&params(), 10, 400, 900);
        assert_eq!(lv.len(), 900);
        assert!(lv[..10].iter().all(|l| *l == (0.0, 0.0)));
        let peak = lv.iter().map(|l| l.0).fold(0.0, f32::max);
        assert!(peak > 0.4, "amp reaches its level, peak {peak}");
        assert!(lv[899].0 < lv[399].0, "release lowers the amp level");
    }

    #[test]
    fn native_r_grows_with_the_envelope_level_and_equals_the_static_law_at_zero() {
        let base = fw_strip_cutoff_r(strip_word_for_cutoff(500.0));
        assert_eq!(native_r(500.0, 0.8, 0.0), base);
        assert_eq!(native_r(500.0, 0.0, 1.0), base);
        assert!(native_r(500.0, 0.8, 1.0) > base);
    }

    #[test]
    fn cutoff_knob_inverts_the_decoder_table() {
        for k in [0usize, 1, 40, 100, 127] {
            assert_eq!(cutoff_knob(FILTER_CUTOFF_HZ_TABLE[k]), k);
        }
    }

    #[test]
    fn flag_reads_the_value_after_the_name() {
        let a: Vec<String> = ["x", "--on", "7"].iter().map(|s| s.to_string()).collect();
        assert_eq!(flag(&a, "--on"), Some("7"));
        assert_eq!(flag(&a, "--off"), None);
    }
}
