//! Test fixtures shared by the `auto_library` submodules.

use crate::loader::{LoadJob, LoadReceiver};
use nord_core::cbin::{stored_archive, ProgramDecoded};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A backup on disk with one organ program at A:11 (bank 0, location 0) and one at A:12.
pub(crate) fn two_program_backup(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("nd-autolib-{name}-{}.ns4b", std::process::id()));
    write_two_program_backup(&path, "First");
    path
}

/// Writes the same layout to `path` with `first` as the name of the A:11 program. A name other
/// than `First` renames the entry too (the stored name is not what the worker reports) and adds
/// an unused entry, so the file length (and with it the file stamp) differs.
pub(crate) fn write_two_program_backup(path: &std::path::Path, first_name: &str) {
    let organ = |name: &str, bars: [u8; 9]| {
        let mut p = ProgramDecoded {
            name: name.into(),
            ..Default::default()
        };
        p.organ.drawbars_a = bars;
        p
    };
    let first = organ(first_name, [8, 8, 8, 0, 0, 0, 0, 0, 0]);
    let second = organ("Second", [0, 0, 0, 0, 0, 0, 0, 0, 8]);
    let a = first.to_cbin_file(0, 0).expect("encode").to_bytes();
    let b = second.to_cbin_file(0, 1).expect("encode").to_bytes();
    let first_entry = format!("Programs/{first_name}.ns4p");
    let mut entries = vec![
        (first_entry.as_str(), &a[..]),
        ("Programs/Second.ns4p", &b[..]),
    ];
    if first_name != "First" {
        entries.push(("Misc/Unused.bin", &b"extra"[..]));
    }
    let bytes = stored_archive(&entries).expect("archive");
    std::fs::write(path, bytes).unwrap();
}

/// Pops up to `n` jobs, waiting at most 10 s for them.
pub(crate) fn drain_until(rx: &LoadReceiver, n: usize) -> Vec<LoadJob> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    while out.len() < n && Instant::now() < deadline {
        match rx.pop() {
            Some(j) => out.push(j),
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::load_queue;

    #[test]
    fn the_fixture_is_a_readable_backup_and_the_drain_stops_at_the_count() {
        let path = two_program_backup("support");
        let archive = nord_core::cbin::Ns4BackupArchive::open(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(archive.entries().len(), 2);
        let (tx, rx) = load_queue(4);
        tx.push(LoadJob::WaveBank(true)).unwrap();
        tx.push(LoadJob::WaveBank(false)).unwrap();
        assert_eq!(drain_until(&rx, 1).len(), 1);
        assert_eq!(drain_until(&rx, 1).len(), 1);
    }
}
