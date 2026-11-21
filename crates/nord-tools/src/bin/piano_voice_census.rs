//! Compares the firmware piano voice list with the engine voices, pedal up and down.
//!
//! Usage: `cargo run --release -p nord-tools --bin piano_voice_census -- <backup.ns4b>`.
//! Loads every `.npno` of the backup and prints, per sound and per class, how many (note,
//! velocity) points each engine path reproduces. Derived numbers only.

use nord_core::cbin::Ns4BackupArchive;
use nord_core::piano::NpnoSound;
use nord_tools::piano_voice_census::{census, report};
use std::fs::File;
use std::sync::Arc;

/// Sound name of an archive entry path (`Piano/Grand/Name.npno` gives `Name`).
fn sound_stem(entry: &str) -> String {
    let file = entry.rsplit('/').next().unwrap_or(entry);
    file.trim_end_matches(".npno").to_string()
}

fn run() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: <backup.ns4b>")?;
    let archive = Ns4BackupArchive::open(&path).map_err(|e| e.to_string())?;
    let mut file = File::open(&path).map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    for e in archive.entries_with_ext(".npno") {
        let bytes = archive
            .read_entry(&mut file, &e.name)
            .map_err(|err| err.to_string())?;
        let Ok(sound) = NpnoSound::from_npno_bytes(&bytes) else {
            eprintln!("skip {}: parse failed", e.name);
            continue;
        };
        if let Some(row) = census(&sound_stem(&e.name), &Arc::new(sound)) {
            rows.push(row);
        }
    }
    rows.sort_by(|a, b| (a.class, &a.name).cmp(&(b.class, &b.name)));
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
    fn sound_stem_drops_the_folders_and_the_extension() {
        assert_eq!(
            sound_stem("Piano/Grand/Velvet Grand Med 6.1.npno"),
            "Velvet Grand Med 6.1"
        );
        assert_eq!(sound_stem("Plain"), "Plain");
    }
}
