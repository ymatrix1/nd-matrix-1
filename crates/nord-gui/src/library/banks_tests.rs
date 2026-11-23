//! Tests for the multi-bank merge (`banks.rs`).
use super::super::support::build_synthetic_zip_stored;
use super::*;
use std::io::Write;

/// `<tmp>/ns4_banks_<pid>_<n>/<tag>.ns4b`: a stable file stem for the bank label.
fn tmp(tag: &str, bytes: &[u8]) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ns4_banks_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{tag}.ns4b"));
    File::create(&p).unwrap().write_all(bytes).unwrap();
    p
}

fn bank(files: &[(&str, &[u8])]) -> Vec<u8> {
    build_synthetic_zip_stored(files)
}

#[test]
fn add_backup_merges_dedupes_and_routes_entries() {
    let a = tmp(
        "a",
        &bank(&[
            ("Program/A/One.ns4p", b"aaaa"),
            ("Samp Lib/K/S.nsmp4", b"ss"),
        ]),
    );
    let b = tmp(
        "b",
        &bank(&[
            ("Program/A/One.ns4p", b"aaaa"),    // identical: skipped
            ("Program/A/Two.ns4p", b"bbbbb"),   // new
            ("Samp Lib/K/S.nsmp4", b"differs"), // conflict: renamed
        ]),
    );
    let mut lib = BackupLibraryState::new();
    let r1 = lib.add_backup_path(&a).unwrap();
    assert!(r1.primary);
    let r2 = lib.add_backup_path(&b).unwrap();
    assert_eq!((r2.added, r2.renamed, r2.duplicates), (1, 1, 1), "{r2:?}");
    assert_eq!(
        lib.programs,
        vec!["Program/A/One.ns4p", "Program/A/Two.ns4p"]
    );
    assert_eq!(
        lib.samples,
        vec!["Samp Lib/K/S [b].nsmp4", "Samp Lib/K/S.nsmp4"]
    );

    // Routing: a renamed key reads the later bank's bytes under the real name.
    let (path, arc, real) = lib.source_of("Samp Lib/K/S [b].nsmp4").unwrap();
    assert_eq!((path, real.as_str()), (b.clone(), "Samp Lib/K/S.nsmp4"));
    let mut rd = BufReader::new(File::open(&b).unwrap());
    assert_eq!(arc.read_entry(&mut rd, &real).unwrap(), b"differs");
    // A primary key still reads the primary bank.
    let (path, _, real) = lib.source_of("Samp Lib/K/S.nsmp4").unwrap();
    assert_eq!((path, real.as_str()), (a.clone(), "Samp Lib/K/S.nsmp4"));
    assert_eq!(lib.bank_of("Program/A/Two.ns4p"), 1);
    assert_eq!(lib.bank_of("Program/A/One.ns4p"), 0);

    for p in [a, b] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn add_same_path_twice_is_a_noop_and_close_resets() {
    let a = tmp("c", &bank(&[("Program/A/One.ns4p", b"x")]));
    let mut lib = BackupLibraryState::new();
    lib.add_backup_path(&a).unwrap();
    let again = lib.add_backup_path(&a).unwrap();
    assert!(again.already_open);
    assert_eq!(lib.bank_paths().len(), 1);
    assert_eq!(lib.programs.len(), 1);

    lib.close_banks();
    assert!(lib.bank_paths().is_empty() && lib.programs.is_empty() && !lib.is_open());
    let _ = std::fs::remove_file(a);
}

#[test]
fn open_backup_path_replaces_merged_banks() {
    let a = tmp("d", &bank(&[("Program/A/One.ns4p", b"x")]));
    let b = tmp("e", &bank(&[("Program/A/Two.ns4p", b"yy")]));
    let mut lib = BackupLibraryState::new();
    lib.add_backup_path(&a).unwrap();
    lib.add_backup_path(&b).unwrap();
    assert_eq!(lib.programs.len(), 2);
    lib.open_backup_path(&a).unwrap();
    assert_eq!(lib.programs, vec!["Program/A/One.ns4p"]);
    assert_eq!(lib.bank_paths().len(), 1);
    for p in [a, b] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn resolve_refs_falls_back_to_other_banks_and_maps_keys() {
    let a = tmp("f", &bank(&[("Program/A/One.ns4p", b"x")]));
    let b = tmp("g", &bank(&[("Piano/Grand/G.npno", b"yy")]));
    let mut lib = BackupLibraryState::new();
    lib.add_backup_path(&a).unwrap();
    lib.add_backup_path(&b).unwrap();
    // Primary knows nothing; bank 1 answers with its real name.
    let refs = lib.resolve_refs(0, |arc| ProgramSoundRefs {
        piano_a: arc
            .entries_with_ext(".npno")
            .first()
            .map(|e| e.name.clone()),
        ..Default::default()
    });
    assert_eq!(refs.piano_a.as_deref(), Some("Piano/Grand/G.npno"));
    assert_eq!(refs.piano_b, None);
    for p in [a, b] {
        let _ = std::fs::remove_file(p);
    }
}
