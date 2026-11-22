//! Level census of the synth layers over the factory programs: one JSON line per program with the
//! RMS level of one note under the engine default and the level change under each named variant,
//! and a final summary line per variant (derived numbers only, no audio).
//!
//! Usage: `synth_census <backup.ns4b> --variant NAME=FLAGS [--variant ...] [--note 60]
//! [--velocity 100] [--limit N] [--only S] [--every K] [--skip N] [--jobs N]`, for example
//! `--variant "uw=--synth-unison-weights"`. Only programs with an enabled synth layer count.
//! Release build only (stack depth in the FX chains). See `docs/research/synth-rows-unison.md`.

use nord_core::cbin::{CbinFile, Ns4BackupArchive};
use nord_core::render::render_timeline;
use nord_core::render_opts::FirmwareOptions;
use nord_tools::rank_metrics::{mono, note_metrics, ANALYSIS_FRAMES};
use nord_tools::sweep_load::{
    build_engine, events, flag, load_sounds, panic_message, PianoCache, SynthCache, FS,
};
use nord_tools::synth_census::{level_delta_db, parse_variant, summarize, Variant};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Frames the note is held (2.08 s). ASSUMED: tool convention shared with `program_rank`.
const HOLD: usize = 100_000;

/// Stack of a render worker thread: the FX chains recurse deeply. ASSUMED: 256 MB is ample.
const WORKER_STACK: usize = 256 << 20;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .ok_or("usage: synth_census <backup.ns4b> --variant NAME=FLAGS [...]")?;
    let num = |name: &str, default: usize| {
        flag(&args, name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let note = num("--note", 60).min(127) as u8;
    let velocity = num("--velocity", 100).min(127) as u8;
    let (limit, every, skip) = (
        num("--limit", usize::MAX),
        num("--every", 1).max(1),
        num("--skip", 0),
    );
    let jobs = num("--jobs", 1).max(1);
    let only = flag(&args, "--only");
    let variants: Vec<Variant> = args
        .windows(2)
        .filter(|w| w[0] == "--variant")
        .map(|w| parse_variant(&w[1]))
        .collect::<Result<_, _>>()?;
    if variants.is_empty() {
        return Err("at least one --variant NAME=FLAGS is needed".into());
    }
    let archive = Ns4BackupArchive::open(path).map_err(|e| e.to_string())?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let (mut pianos, mut synths): (PianoCache, SynthCache) = (HashMap::new(), HashMap::new());
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
    let mut deltas: Vec<Vec<f64>> = vec![Vec::new(); variants.len()];
    for name in names {
        let bytes = archive
            .read_entry(&mut file, &name)
            .map_err(|e| e.to_string())?;
        let prog = CbinFile::parse(&bytes)
            .and_then(|c| c.decode_program())
            .map_err(|e| format!("{name}: {e}"))?;
        if !(prog.synth.layer_a.enabled || prog.synth.layer_b.enabled || prog.synth.layer_c.enabled)
        {
            continue;
        }
        let refs = archive.resolve_program_sounds(&prog);
        load_sounds(&archive, &mut file, &refs, &mut pianos, &mut synths);
        let run = |o: &FirmwareOptions| -> Result<f64, String> {
            catch_unwind(AssertUnwindSafe(|| {
                let mut engine = build_engine(&prog, &refs, &pianos, &synths, o);
                let ev = events(&[note], velocity, HOLD);
                let (l, r) = render_timeline(&mut engine, &ev, ANALYSIS_FRAMES);
                note_metrics(&mono(&l, &r), f64::from(FS)).rms_db
            }))
            .map_err(panic_message)
        };
        let mut all: Vec<(String, FirmwareOptions)> =
            vec![("default".into(), FirmwareOptions::default())];
        all.extend(variants.iter().map(|v| (v.name.clone(), v.opts)));
        let next = std::sync::atomic::AtomicUsize::new(0);
        let out: std::sync::Mutex<Vec<(usize, Result<f64, String>)>> = Default::default();
        std::thread::scope(|s| {
            for _ in 0..jobs.min(all.len()) {
                std::thread::Builder::new()
                    .stack_size(WORKER_STACK)
                    .spawn_scoped(s, || loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((_, o)) = all.get(i) else { break };
                        let r = run(o);
                        out.lock().unwrap().push((i, r));
                    })
                    .expect("spawn render worker");
            }
        });
        let mut res = out.into_inner().unwrap();
        res.sort_by_key(|(i, _)| *i);
        let levels: Vec<Result<f64, String>> = res.into_iter().map(|(_, r)| r).collect();
        let base = levels[0].clone().unwrap_or(f64::NAN);
        let mut line = json!({ "entry": name, "name": prog.name, "default_db": base });
        for (k, (label, _)) in all.iter().enumerate().skip(1) {
            let d = levels[k]
                .as_ref()
                .map_or(f64::NAN, |&x| level_delta_db(base, x));
            line[label] = json!(d);
            deltas[k - 1].push(d);
        }
        println!("{line}");
    }
    let summary: serde_json::Map<String, Value> = variants
        .iter()
        .zip(&deltas)
        .map(|(v, d)| {
            let s = summarize(d).map(|s| {
                json!({ "n": s.n, "median_db": s.median_db, "mean_abs_db": s.mean_abs_db,
                        "p10_db": s.p10_db, "p90_db": s.p90_db, "within_3_db": s.within_3_db })
            });
            (v.name.clone(), s.unwrap_or(Value::Null))
        })
        .collect();
    println!("{}", json!({ "summary": summary }));
    Ok(())
}

/// The release tail of the note is part of the analysis window.
const _: () = assert!(HOLD < ANALYSIS_FRAMES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_parse_like_the_library_and_levels_subtract_in_db() {
        let v = parse_variant("oracle=--synth-oracle").unwrap();
        assert!(v.opts.synth_unison_weights);
        assert!(level_delta_db(-30.0, -33.0) < 0.0);
    }
}
