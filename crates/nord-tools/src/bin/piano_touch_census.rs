//! Prints the Piano Touch census of every `.ns4p` program of a Nord Stage 4 backup.
//!
//! Usage: `cargo run --release -p nord-tools --bin piano_touch_census -- <backup.ns4b> [--csv]`.
//! `--csv` prints one row per program (entry, slot, name, layer Touch edit values, in-use flags)
//! for `tools/arm_program_touch_bits.py`.
//! Derived numbers and program names only.

use nord_core::cbin::{CbinFile, Ns4BackupArchive};
use nord_tools::touch_census::{csv_row, facts, report, CSV_HEADER};
use std::fs::File;

/// Program name of an archive entry path (`Program/Bank A/Name.ns4p` gives `Name`).
fn program_stem(entry: &str) -> String {
    let file = entry.rsplit('/').next().unwrap_or(entry);
    file.trim_end_matches(".ns4p").to_string()
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: <backup.ns4b> [--csv]")?;
    let csv = args.next().as_deref() == Some("--csv");
    let archive = Ns4BackupArchive::open(&path).map_err(|e| e.to_string())?;
    let mut file = File::open(&path).map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    let mut entries = Vec::new();
    for e in archive.entries_with_ext(".ns4p") {
        let bytes = archive
            .read_entry(&mut file, &e.name)
            .map_err(|err| err.to_string())?;
        let prog = CbinFile::parse(&bytes)
            .and_then(|c| c.decode_program())
            .map_err(|err| format!("{}: {err}", e.name))?;
        let mut row = facts(&prog);
        row.sound_a = archive.resolve_program_sounds(&prog).piano_a;
        row.name = program_stem(&e.name);
        entries.push(e.name.clone());
        rows.push(row);
    }
    if csv {
        let mut lines: Vec<String> = entries
            .iter()
            .zip(&rows)
            .map(|(e, r)| csv_row(e, r))
            .collect();
        lines.sort();
        println!("{CSV_HEADER}");
        for l in lines {
            println!("{l}");
        }
        return Ok(());
    }
    rows.sort_by(|a, b| a.slot.cmp(&b.slot));
    print!("{}", report(&rows));
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_stem_drops_the_folders_and_the_extension() {
        assert_eq!(
            program_stem("Program/Bank A/Pearl Upright.ns4p"),
            "Pearl Upright"
        );
        assert_eq!(program_stem("Plain"), "Plain");
    }
}
