//! Reading the saved `.ns4b` for the worker: the program entry at each location, entry bytes,
//! the decoded program and the sounds it needs. Nothing here touches the audio thread.

use nord_core::cbin::{CbinFile, Ns4BackupArchive, ProgramDecoded, SlotIndex};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// A sound a program needs, by archive entry; `None` when the layer is off.
pub(super) type Wanted = Option<String>;

/// The opened library: its archive, the program entry at each flat location
/// (`bank * 64 + location`), and a stamp of the file it was read from.
pub(super) struct Indexed {
    pub archive: Ns4BackupArchive,
    pub slots: Vec<(u16, String)>,
    /// Modification time and size of the file, so a library replaced on disk while the host
    /// runs does not serve banks decoded from the old file (`bank_key`).
    pub stamp: String,
}

/// Indexes the archive at `path`.
pub(super) fn index(path: &Path) -> Result<Indexed, String> {
    let archive = Ns4BackupArchive::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let stamp = stamp_of(file.metadata().ok());
    let mut reader = BufReader::new(file);
    let slots = SlotIndex::read(&archive, &mut reader)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .programs
        .into_iter()
        .filter(|(b, l, _)| *b < 8 && *l < 64)
        .map(|(b, l, n)| (b * 64 + l, n))
        .collect();
    Ok(Indexed {
        archive,
        slots,
        stamp,
    })
}

/// The stamp of the file at `path` now (no read of its content), to notice a library replaced
/// on disk since [`index`] ran.
pub(super) fn current_stamp(path: &Path) -> String {
    stamp_of(std::fs::metadata(path).ok())
}

/// `<seconds since the epoch of the last write>-<length>`; `0-0` when the file says nothing.
fn stamp_of(meta: Option<std::fs::Metadata>) -> String {
    let secs = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let len = meta.map_or(0, |m| m.len());
    format!("{secs}-{len}")
}

/// The process-wide key of a decoded bank (`shared_sounds.rs`): the library file, its stamp
/// and the entry.
pub(super) fn bank_key(path: &Path, stamp: &str, entry: &str) -> String {
    format!("{}|{stamp}|{entry}", path.display())
}

pub(super) fn read_entry(
    path: &Path,
    archive: &Ns4BackupArchive,
    entry: &str,
) -> Result<Vec<u8>, String> {
    let mut r = BufReader::new(File::open(path).map_err(|e| format!("{}: {e}", path.display()))?);
    archive.read_entry(&mut r, entry).map_err(|e| e.to_string())
}

/// The program entry decoded, named after its file when the stored name is the generic one
/// (the same rule as the editor's library).
pub(super) fn decode_program(
    path: &Path,
    archive: &Ns4BackupArchive,
    entry: &str,
) -> Result<ProgramDecoded, String> {
    let bytes = read_entry(path, archive, entry)?;
    let mut prog = CbinFile::parse(&bytes)
        .and_then(|c| c.decode_program())
        .map_err(|e| e.to_string())?;
    let generic = prog.name.is_empty()
        || prog.name == "Nord Stage 4 Program"
        || prog.name.ends_with(" Program");
    if generic {
        prog.name = short_name(entry).to_string();
    }
    Ok(prog)
}

/// The piano (A, B) and sample (A, B, C) entries `prog` needs: a layer that is enabled, or is in
/// a scene of the program, or when the program has no scene masks at all (the editor's rule).
pub(super) fn wanted_sounds(
    archive: &Ns4BackupArchive,
    prog: &ProgramDecoded,
) -> ([Wanted; 2], [Wanted; 3]) {
    let refs = archive.resolve_program_sounds(prog);
    let mask = prog.global.scene_1_mask | prog.global.scene_2_mask;
    let needed = |enabled: bool, bit: u8| enabled || mask == 0 || mask & (1 << bit) != 0;
    let pick =
        |enabled: bool, bit: u8, entry: Option<String>| entry.filter(|_| needed(enabled, bit));
    (
        [
            pick(prog.piano.layer_a.enabled, 2, refs.piano_a),
            pick(prog.piano.layer_b.enabled, 3, refs.piano_b),
        ],
        [
            pick(prog.synth.layer_a.enabled, 4, refs.synth_a),
            pick(prog.synth.layer_b.enabled, 5, refs.synth_b),
            pick(prog.synth.layer_c.enabled, 6, refs.synth_c),
        ],
    )
}

pub(super) fn short_name(entry: &str) -> &str {
    Path::new(entry)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Program")
}

#[cfg(test)]
mod tests {
    use super::super::support::two_program_backup;
    use super::*;

    #[test]
    fn the_index_lists_the_programs_by_flat_location_and_stamps_the_file() {
        let path = two_program_backup("index");
        let lib = index(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(lib.slots.len(), 2);
        assert_eq!(lib.slots[0].0, 0);
        assert_eq!(lib.slots[1].0, 1);
        assert_eq!(lib.slots[0].1, "Programs/First.ns4p");
        assert!(
            !lib.stamp.ends_with("-0"),
            "the size is in the stamp: {}",
            lib.stamp
        );
        assert!(index(Path::new("definitely-missing.ns4b")).is_err());
    }

    #[test]
    fn a_decoded_program_keeps_its_name_and_the_default_program_wants_no_sounds() {
        let path = two_program_backup("decode");
        let lib = index(&path).unwrap();
        let prog = decode_program(&path, &lib.archive, "Programs/Second.ns4p").unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(prog.name, "Second");
        let (p, s) = wanted_sounds(&lib.archive, &ProgramDecoded::default());
        assert!(p.iter().all(Option::is_none) && s.iter().all(Option::is_none));
    }

    #[test]
    fn the_bank_key_names_the_file_its_stamp_and_the_entry() {
        assert_eq!(
            bank_key(Path::new("lib.ns4b"), "7-9", "Pianos/x.npno"),
            "lib.ns4b|7-9|Pianos/x.npno"
        );
        assert_eq!(short_name("Programs/Grand Piano.ns4p"), "Grand Piano");
        assert_eq!(short_name(""), "Program");
    }
}
