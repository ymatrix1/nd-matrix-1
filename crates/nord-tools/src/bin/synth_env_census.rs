//! Envelope index census of the factory synth layers (D212): one JSON line per enabled synth layer
//! with the stored amp and filter rate indexes (`knob - 64`, decay knob 127 is Hold), and a final
//! summary line with the program and layer counts.
//!
//! Usage: `cargo run --release -p nord-tools --bin synth_env_census -- <dir of .ns4p>`; the
//! directory is walked recursively (`factory_programs/Program`, 384 programs). Derived numbers
//! only. The bucket counts of `docs/research/synth-env-rates-settled.md` come from
//! `python tools/synth_env_rates.py census`.

use nord_core::cbin::{CbinFile, ProgramDecoded};
use nord_core::engine::Stage4Engine;
use nord_core::synth::{rate_index_for_knob, EnvKnobs, StripKnobs};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn env_json(k: EnvKnobs) -> Value {
    json!({
        "attack": rate_index_for_knob(k.attack),
        "decay": rate_index_for_knob(k.decay),
        "release": rate_index_for_knob(k.release),
        "hold": k.decay >= 127,
    })
}

/// JSON row of one layer's stored knobs.
pub fn layer_row(program: &str, layer: usize, k: &StripKnobs) -> Value {
    json!({
        "program": program,
        "layer": layer,
        "amp": env_json(k.amp),
        "filter": env_json(k.filter),
    })
}

/// Rows of the enabled synth layers of a program that carry envelope knobs.
pub fn program_rows(name: &str, prog: &ProgramDecoded) -> Vec<Value> {
    let mut engine = Stage4Engine::new(48_000.0);
    engine.apply_program(prog);
    let layers = [
        (prog.synth.layer_a.enabled, &*engine.synth_a),
        (prog.synth.layer_b.enabled, &*engine.synth_b),
        (prog.synth.layer_c.enabled, &*engine.synth_c),
    ];
    layers
        .iter()
        .enumerate()
        .filter(|(_, (on, _))| *on)
        .filter_map(|(i, (_, l))| l.strip_knobs.map(|k| layer_row(name, i, &k)))
        .collect()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "ns4p") {
            out.push(p);
        }
    }
    Ok(())
}

fn run(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    walk(root, &mut files).map_err(|e| format!("{}: {e}", root.display()))?;
    files.sort();
    let (mut programs, mut layers) = (0usize, 0usize);
    for f in &files {
        let bytes = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let prog = CbinFile::parse(&bytes)
            .and_then(|c| c.decode_program())
            .map_err(|e| format!("{}: {e}", f.display()))?;
        let name = f
            .strip_prefix(root)
            .unwrap_or(f)
            .with_extension("")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let rows = program_rows(&name, &prog);
        programs += usize::from(!rows.is_empty());
        layers += rows.len();
        for r in rows {
            println!("{r}");
        }
    }
    println!(
        "{}",
        json!({"summary": {"files": files.len(), "synth_programs": programs, "layers": layers}})
    );
    Ok(())
}

fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: synth_env_census <dir of .ns4p>");
        std::process::exit(2);
    };
    if let Err(e) = run(Path::new(&dir)) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_carry_the_rate_index_and_hold() {
        let e = EnvKnobs {
            attack: 0,
            decay: 127,
            release: 100,
        };
        let r = layer_row("p", 1, &StripKnobs { amp: e, filter: e });
        assert_eq!(r["layer"], 1);
        assert_eq!(r["amp"]["attack"], -64);
        assert_eq!(r["amp"]["decay"], 63);
        assert_eq!(r["amp"]["hold"], true);
        assert_eq!(r["filter"]["release"], 36);
    }

    #[test]
    fn a_default_program_has_no_enabled_synth_layer_rows_without_knobs() {
        let rows = program_rows("x", &ProgramDecoded::default());
        assert!(rows.iter().all(|r| r["layer"].as_u64().unwrap() < 3));
    }

    #[test]
    fn walk_finds_only_ns4p_files() {
        let dir = std::env::temp_dir().join(format!("ns4_census_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.ns4p"), b"").unwrap();
        std::fs::write(dir.join("sub/b.ns4p"), b"").unwrap();
        std::fs::write(dir.join("c.txt"), b"").unwrap();
        let mut f = Vec::new();
        walk(&dir, &mut f).unwrap();
        assert_eq!(f.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
