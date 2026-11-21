//! Native side of the program ranking (`docs/research/program-ranking.md`): renders every factory
//! program of a backup for ONE held note (default key 60, velocity 100, 2.08 s held, 2.73 s
//! analysed) twice, with the engine defaults and with the firmware-derived opt-ins
//! (`--firmware-all --firmware-routing`, the oracle-verified model), and prints one JSON line per
//! program: engines in scene I and II, layer counts, and level / centroid / band numbers per mode.
//! `--ablate` adds one more render per option group (the firmware options with that group put back
//! to the engine default), which says which group carries the distance. `--ablate-fx` renders every
//! non-empty combination of the six FX groups put back to the default (63 renders, `--jobs N`
//! threads), for the pairs and the best set that `tools/program_rank_combo.py` derives. Numbers only; no audio,
//! firmware or sample content. `--legacy-mod2` ranks against the engine default before D163 (firmware
//! Mod 2 off), which reproduces the committed ranking data. `--flip SETS` (`fx_mod1+fx_mod2,fx_eq`, `all-fx`, or `every-fx` for
//! all 63 combinations) renders the engine default with those groups switched to the firmware value:
//! what a default flip of the group would sound like, to compare against the firmware render.
//!
//! Usage: `cargo run --release -p nord-tools --bin program_rank -- <backup.ns4b> [--note 60]
//! [--velocity 100] [--limit N] [--only S] [--every K] [--ablate] [--ablate-fx] [--flip SETS|every-fx] [--legacy-mod2] [--legacy-fx-ports] [--jobs N]`. Release build only (stack depth in the FX
//! chains). `tools/program_rank.py` joins the lines with the chain census.

use nord_core::cbin::{CbinFile, Ns4BackupArchive, ProgramDecoded, ProgramSoundRefs};
use nord_core::render::render_timeline;
use nord_core::render_opts::FirmwareOptions;
use nord_tools::rank_groups::{
    baseline, firmware_options, flipped_set, fx_subsets, parse_sets, reverted, reverted_set,
    subset_key, GROUPS,
};
use nord_tools::rank_metrics::{mono, note_metrics, NoteMetrics, ANALYSIS_FRAMES};
use nord_tools::sweep_load::{
    build_engine, events, flag, load_sounds, panic_message, PianoCache, SynthCache, FS,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Frames the note is held (2.08 s), the rest of the analysis window is the release.
/// ASSUMED: a tool convention (long enough for the attack, decay and the FX to settle for every
/// factory program); nothing in the firmware or a recording fixes it. The ranking only compares two
/// renders with the same hold, so a different value moves every score alike.
const HOLD: usize = 100_000;

/// Sections enabled in a scene from `section_enable_*` (bit 2 organ, bit 1 piano, bit 0 synth).
fn sections(bits: u8) -> Value {
    json!({ "organ": bits & 4 != 0, "piano": bits & 2 != 0, "synth": bits & 1 != 0 })
}

/// Layer census of a program: enabled layers per engine and how many synth layers use a SAMPLES
/// multisample (a reference the backup resolved).
fn layers(prog: &ProgramDecoded, refs: &ProgramSoundRefs) -> Value {
    let piano = [prog.piano.layer_a.enabled, prog.piano.layer_b.enabled];
    let synth = [
        (prog.synth.layer_a.enabled, refs.synth_a.is_some()),
        (prog.synth.layer_b.enabled, refs.synth_b.is_some()),
        (prog.synth.layer_c.enabled, refs.synth_c.is_some()),
    ];
    json!({
        "piano": piano.iter().filter(|e| **e).count(),
        "synth": synth.iter().filter(|(e, _)| *e).count(),
        "synth_sample": synth.iter().filter(|(e, s)| *e && *s).count(),
        "organ_model": format!("{:?}", prog.organ.model),
    })
}

/// Renders `prog` once with `opts` and measures the mono mean.
fn measure(
    prog: &ProgramDecoded,
    refs: &ProgramSoundRefs,
    pianos: &PianoCache,
    synths: &SynthCache,
    opts: &FirmwareOptions,
    note: u8,
    velocity: u8,
) -> Result<NoteMetrics, String> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut engine = build_engine(prog, refs, pianos, synths, opts);
        let ev = events(&[note], velocity, HOLD);
        let (l, r) = render_timeline(&mut engine, &ev, ANALYSIS_FRAMES);
        note_metrics(&mono(&l, &r), f64::from(FS))
    }))
    .map_err(panic_message)
}

/// Stack of a render worker thread: the FX chains recurse deeply (the main thread has 8 MB or
/// more on the release build). ASSUMED: 256 MB is ample; only address space is reserved.
const WORKER_STACK: usize = 256 << 20;

/// `f` over every item with up to `jobs` threads; results in item order.
fn run_parallel<T: Sync, R: Send>(items: &[T], jobs: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out: std::sync::Mutex<Vec<(usize, R)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs.min(items.len()).max(1) {
            std::thread::Builder::new()
                .stack_size(WORKER_STACK)
                .spawn_scoped(s, || loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    out.lock().unwrap().push((i, r));
                })
                .expect("spawn render worker");
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, r)| r).collect()
}

fn metrics_json(m: &Result<NoteMetrics, String>) -> Value {
    match m {
        Ok(m) => json!({
            "rms_db": m.rms_db, "peak_db": m.peak_db,
            "centroid_hz": m.centroid_hz, "bands_db": m.bands_db,
        }),
        Err(p) => json!({ "panic": p }),
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .ok_or("usage: program_rank <backup.ns4b> [--note N] [--velocity V] [--limit N] [--only S] [--every K]")?;
    let num = |name: &str, default: usize| {
        flag(&args, name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let note = num("--note", 60).min(127) as u8;
    let velocity = num("--velocity", 100).min(127) as u8;
    let (limit, every) = (num("--limit", usize::MAX), num("--every", 1).max(1));
    let only = flag(&args, "--only");
    let ablate = args.iter().any(|a| a == "--ablate");
    let ablate_fx = args.iter().any(|a| a == "--ablate-fx");
    let jobs = num("--jobs", 1).max(1);
    let flips: Vec<Vec<usize>> = match flag(&args, "--flip") {
        Some(list) if list == "every-fx" => fx_subsets(),
        Some(list) => parse_sets(list)?,
        None => Vec::new(),
    };
    let archive = Ns4BackupArchive::open(path).map_err(|e| e.to_string())?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let (mut pianos, mut synths): (PianoCache, SynthCache) = (HashMap::new(), HashMap::new());
    let names: Vec<String> = archive
        .entries_with_ext(".ns4p")
        .iter()
        .map(|e| e.name.clone())
        .filter(|n| only.is_none_or(|s| n.contains(s.as_str())))
        .step_by(every)
        .take(limit)
        .collect();
    std::panic::set_hook(Box::new(|_| {}));
    let firmware = firmware_options();
    let base = baseline(
        args.iter().any(|a| a == "--legacy-mod2"),
        args.iter().any(|a| a == "--legacy-fx-ports"),
    );
    for name in names {
        let bytes = archive
            .read_entry(&mut file, &name)
            .map_err(|e| e.to_string())?;
        let prog = CbinFile::parse(&bytes)
            .and_then(|c| c.decode_program())
            .map_err(|e| format!("{name}: {e}"))?;
        let refs = archive.resolve_program_sounds(&prog);
        load_sounds(&archive, &mut file, &refs, &mut pianos, &mut synths);
        let run = |o: &FirmwareOptions| measure(&prog, &refs, &pianos, &synths, o, note, velocity);
        let default = run(&base);
        let fw = run(&firmware);
        let mut line = json!({
            "entry": name,
            "name": prog.name,
            "scene_1": sections(prog.global.section_enable_i),
            "scene_2": sections(prog.global.section_enable_ii),
            "layers": layers(&prog, &refs),
            "note": note,
            "velocity": velocity,
            "default": metrics_json(&default),
            "firmware": metrics_json(&fw),
        });
        if !flips.is_empty() {
            let results = run_parallel(&flips, jobs, |set| {
                metrics_json(&run(&flipped_set(&firmware, &base, set)))
            });
            let map: serde_json::Map<String, Value> = flips
                .iter()
                .zip(results)
                .map(|(set, m)| (subset_key(set), m))
                .collect();
            line["flip"] = Value::Object(map);
        }
        if ablate_fx {
            let sets = fx_subsets();
            let results = run_parallel(&sets, jobs, |set| {
                metrics_json(&run(&reverted_set(&firmware, &base, set)))
            });
            let map: serde_json::Map<String, Value> = sets
                .iter()
                .zip(results)
                .map(|(set, m)| (subset_key(set), m))
                .collect();
            line["ablate_fx"] = Value::Object(map);
        }
        if ablate {
            let ab: serde_json::Map<String, Value> = (0..GROUPS.len())
                .map(|g| {
                    (
                        GROUPS[g].0.to_string(),
                        metrics_json(&run(&reverted(&firmware, &base, g))),
                    )
                })
                .collect();
            line["ablate"] = Value::Object(ab);
        }
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_decode_the_enable_bits() {
        let s = sections(0b101);
        assert_eq!(s["organ"], true);
        assert_eq!(s["piano"], false);
        assert_eq!(s["synth"], true);
    }

    #[test]
    fn layers_census_counts_enabled_layers_of_the_default_program() {
        let prog = ProgramDecoded::default();
        let l = layers(&prog, &ProgramSoundRefs::default());
        assert_eq!(l["synth_sample"], 0);
        assert!(l["organ_model"].as_str().is_some());
    }

    #[test]
    fn measure_default_program_is_finite_and_serialises() {
        let prog = ProgramDecoded::default();
        let refs = ProgramSoundRefs::default();
        let m = measure(
            &prog,
            &refs,
            &HashMap::new(),
            &HashMap::new(),
            &FirmwareOptions::default(),
            60,
            100,
        );
        let v = metrics_json(&m);
        assert!(v["rms_db"].is_number(), "{v}");
        assert_eq!(v["bands_db"].as_array().map(Vec::len), Some(8));
    }

    #[test]
    fn run_parallel_keeps_item_order_for_any_job_count() {
        let items: Vec<u32> = (0..20).collect();
        for jobs in [1, 3, 64] {
            let r = run_parallel(&items, jobs, |x| x * 2);
            assert_eq!(r, (0..20).map(|x| x * 2).collect::<Vec<_>>());
        }
        assert!(run_parallel(&Vec::<u32>::new(), 4, |x| *x).is_empty());
    }

    #[test]
    fn panic_is_reported_not_propagated() {
        let v = metrics_json(&Err("boom".into()));
        assert_eq!(v["panic"], "boom");
    }
}
