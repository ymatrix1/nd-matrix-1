//! Native side of `tools/synth_env_trace.py` (D195): the strip envelope chains and the cutoff
//! the native engine derives from them, one value per firmware pass, for one synth layer of a
//! program (`.ns4p`).
//!
//! Usage: `cargo run --release -p nord-tools --bin synth_env_native -- <program.ns4p> [--layer 0|1|2]
//! [--on 300] [--off 11149] [--passes 16575]`. Prints one JSON line: the stored knobs, the
//! record words, the panel cutoff and envelope amount, and per pass the amp level, the filter
//! level, and the native cutoff coefficient `R` (strip word law of `fw_strip_cutoff.rs`) after
//! the engine's `2^(amount x octaves x level)` filter envelope law. No audio is produced.
//! `--adr-law [--g 98]` applies the opt-in oracle envelope law of D210 (amp attack scaled by the
//! key byte `g`, settled attack table, decay sustain 0.5). `--rates settled` reads the settled
//! rate tables (D212, `fw_env_settled.rs`) instead of the stale sweep, `--sustain-half` stops both
//! decay records at 0.5 (the oracle image default) without the velocity law, so a table error can
//! be compared alone.

use nord_core::cbin::{CbinFile, FILTER_CUTOFF_HZ_TABLE};
use nord_core::engine::Stage4Engine;
use nord_core::synth::{
    fw_env_cutoff_hz, fw_strip_cutoff_r, rate_index_for_knob, strip_word_for_cutoff, RateTables,
    StripEnvRecords, StripKnobs, StripParams, StripVoice, SynthLayer, DEFAULT_TABLES,
    SETTLED_TABLES,
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
pub fn chain_levels(
    p: &StripParams,
    mut v: StripVoice,
    on: usize,
    off: usize,
    passes: usize,
) -> Vec<(f32, f32)> {
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

/// Key tracking factor of a note offset from the reference note 60 (`kb_track` 0..=3 is
/// `{0, 1/3, 2/3, 1}` semitone per semitone, VERIFIED `synth-cutoff-voice-law.md`).
pub fn kb_scale(kb_track: u8, note_offset: f32) -> f32 {
    2.0f32.powf(note_offset / 12.0 * f32::from(kb_track.min(3)) / 3.0)
}

/// Effective cutoff in Hz per pass for both envelope laws: the default `2^(amount * 8 * level)`
/// and the opt-in firmware semitone law (`fw_env_cutoff_hz`, D200), key tracking applied first.
pub fn cutoff_hz_pair(base_hz: f32, amount: f32, level: f32) -> (f32, f32) {
    let default = base_hz * 2.0f32.powf(amount.clamp(-1.0, 1.0) * NATIVE_ENV_OCTAVES * level);
    (default, fw_env_cutoff_hz(base_hz, amount, level))
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

/// Rate tables and the sustain override of a run (`--rates`, `--sustain-half`).
#[derive(Clone, Copy)]
pub struct Variant {
    pub tables: RateTables,
    pub sustain_half: bool,
}

impl Variant {
    /// Default tables, panel sustain.
    pub fn default_run() -> Self {
        Self {
            tables: DEFAULT_TABLES,
            sustain_half: false,
        }
    }
}

/// Chain parameters and the voice of a layer; `law = Some(g)` applies the D210 oracle law.
pub fn chain_setup(
    knobs: &StripKnobs,
    sustain: f32,
    law: Option<u8>,
    variant: Variant,
) -> (StripParams, StripVoice) {
    let mut params = StripParams {
        amp: StripEnvRecords::from_knobs_with(knobs.amp, sustain, variant.tables),
        filter: StripEnvRecords::from_knobs_with(knobs.filter, 0.0, variant.tables),
        period_frames: 1.0,
    };
    if variant.sustain_half {
        params.amp.decay.thr = 0.5;
        params.filter.decay.thr = 0.5;
    }
    let Some(g) = law else {
        return (params, StripVoice::default());
    };
    let word = rate_index_for_knob(knobs.amp.attack);
    params.amp = params.amp.with_oracle_law(word);
    params.filter = params
        .filter
        .with_oracle_law(rate_index_for_knob(knobs.filter.attack));
    (params, StripVoice::with_scaled_amp_attack(word, g))
}

#[allow(clippy::too_many_arguments)] // one flat call per CLI flag
fn layer_report(
    layer: &SynthLayer,
    on: usize,
    off: usize,
    passes: usize,
    note_offset: f32,
    kb_track: u8,
    law: Option<u8>,
    variant: Variant,
) -> Result<Value, String> {
    let knobs = layer
        .strip_knobs
        .ok_or("program carries no envelope knobs")?;
    let (params, voice) = chain_setup(&knobs, layer.sustain_level, law, variant);
    let lv = chain_levels(&params, voice, on, off, passes);
    let amount = layer.filter_env_amount;
    let r: Vec<f32> = lv
        .iter()
        .map(|&(_, f)| native_r(layer.cutoff_hz, amount, f))
        .collect();
    // The key offset reaches the index sum at the key-on and stays after the key-off (oracle
    // `DM(0xC2717)` keeps the last note offset).
    let kb = kb_scale(kb_track, note_offset);
    let hz: Vec<(f32, f32)> = lv
        .iter()
        .enumerate()
        .map(|(n, &(_, f))| {
            let base = if n < on {
                layer.cutoff_hz
            } else {
                layer.cutoff_hz * kb
            };
            cutoff_hz_pair(base, amount, f)
        })
        .collect();
    Ok(json!({
        "note_offset": note_offset,
        "kb_track_used": kb_track,
        "filter_ladder": layer.filter_model == nord_core::synth::FilterModel::Ladder24,
        "hz_default": hz.iter().map(|h| h.0).collect::<Vec<_>>(),
        "hz_fw": hz.iter().map(|h| h.1).collect::<Vec<_>>(),
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
    let note_offset = flag(args, "--note-offset").map_or(Ok(0.0), |v| {
        v.parse::<f32>()
            .map_err(|_| "bad --note-offset".to_string())
    })?;
    // The oracle injects no key tracking word: its offset passes at unity (`--kb-track 3`).
    let kb_track = num("--kb-track", usize::from(layer.kb_track))?.min(3) as u8;
    let law = args
        .iter()
        .any(|a| a == "--adr-law")
        .then(|| num("--g", 98))
        .transpose()?
        .map(|g| g.min(127) as u8);
    let variant = Variant {
        tables: match flag(args, "--rates") {
            None | Some("legacy") => DEFAULT_TABLES,
            Some("settled") => SETTLED_TABLES,
            Some(_) => return Err("--rates must be legacy or settled".into()),
        },
        sustain_half: args.iter().any(|a| a == "--sustain-half"),
    };
    println!(
        "{}",
        layer_report(layer, on, off, passes, note_offset, kb_track, law, variant)?
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
        let lv = chain_levels(&params(), StripVoice::default(), 10, 400, 900);
        assert_eq!(lv.len(), 900);
        assert!(lv[..10].iter().all(|l| *l == (0.0, 0.0)));
        let peak = lv.iter().map(|l| l.0).fold(0.0, f32::max);
        assert!(peak > 0.4, "amp reaches its level, peak {peak}");
        assert!(lv[899].0 < lv[399].0, "release lowers the amp level");
    }

    #[test]
    fn adr_law_setup_scales_the_amp_attack_and_stops_decay_at_half() {
        let e = EnvKnobs {
            attack: 4,
            decay: 60,
            release: 50,
        };
        let k = StripKnobs { amp: e, filter: e };
        let (p0, v0) = chain_setup(&k, 0.0, None, Variant::default_run());
        let (p1, v1) = chain_setup(&k, 0.0, Some(20), Variant::default_run());
        assert!(v0.amp_attack_b.is_none() && v1.amp_attack_b.is_some());
        assert!(
            v1.amp_attack_b.unwrap() < p1.amp.attack.b,
            "soft key is slower"
        );
        assert_eq!(
            (p0.amp.decay.thr, p1.amp.decay.thr, p1.filter.decay.thr),
            (0.0, 0.5, 0.5)
        );
        let (_, v127) = chain_setup(&k, 0.0, Some(127), Variant::default_run());
        assert_eq!(
            v127.amp_attack_b,
            Some(p1.amp.attack.b),
            "g = 127 is unscaled"
        );
    }

    #[test]
    fn settled_rates_and_sustain_half_change_only_the_records() {
        let e = EnvKnobs {
            attack: 10,
            decay: 40,
            release: 30,
        };
        let k = StripKnobs { amp: e, filter: e };
        let (def, _) = chain_setup(&k, 0.0, None, Variant::default_run());
        let settled = Variant {
            tables: SETTLED_TABLES,
            sustain_half: true,
        };
        let (set, v) = chain_setup(&k, 0.0, None, settled);
        assert!(v.amp_attack_b.is_none(), "no velocity law");
        assert_ne!(def.amp.attack.b, set.amp.attack.b);
        assert_ne!(def.amp.decay.a, set.amp.decay.a);
        assert_eq!((set.amp.decay.thr, set.filter.decay.thr), (0.5, 0.5));
        assert_eq!(def.amp.decay.thr, 0.0);
    }

    #[test]
    fn native_r_grows_with_the_envelope_level_and_equals_the_static_law_at_zero() {
        let base = fw_strip_cutoff_r(strip_word_for_cutoff(500.0));
        assert_eq!(native_r(500.0, 0.8, 0.0), base);
        assert_eq!(native_r(500.0, 0.0, 1.0), base);
        assert!(native_r(500.0, 0.8, 1.0) > base);
    }

    #[test]
    fn key_tracking_moves_the_cutoff_by_a_third_of_the_offset_per_step() {
        assert_eq!(kb_scale(0, -12.0), 1.0);
        assert!((kb_scale(3, -12.0) - 0.5).abs() < 1e-6);
        assert!((kb_scale(1, 12.0) - 2.0f32.powf(1.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn firmware_law_sweeps_further_than_the_default_at_full_level() {
        let (def, fw) = cutoff_hz_pair(200.0, 0.5, 1.0);
        assert!((def / 200.0 - 16.0).abs() < 1e-3, "{def}");
        assert!(
            (fw / 200.0 / (63.5f32 / 12.0).exp2() - 1.0).abs() < 1e-3,
            "{fw}"
        );
        assert_eq!(cutoff_hz_pair(200.0, 0.5, 0.0), (200.0, 200.0));
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
