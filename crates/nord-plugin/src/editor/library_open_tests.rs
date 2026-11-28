//! Cost of opening the saved library when the editor is created (`EditorState::new` runs
//! `plugin_library` on the host's UI thread; D252 documents it instead of moving it).

use super::draw::plugin_library;
use nord_core::cbin::{stored_archive, ProgramDecoded};
use std::time::Instant;

/// A synthetic library of `programs` organ programs plus `sounds` small sound entries (stored,
/// so building it is quick; names follow the real layout).
fn synthetic_library(name: &str, programs: usize, sounds: usize) -> std::path::PathBuf {
    let prog = ProgramDecoded {
        name: "P".into(),
        ..Default::default()
    };
    let bytes = prog.to_cbin_file(0, 0).expect("encode").to_bytes();
    let mut names: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..programs {
        names.push((format!("Programs/P{i:04}.ns4p"), bytes.clone()));
    }
    for i in 0..sounds {
        names.push((format!("Sounds/S{i:05}.nsmp4"), vec![0u8; 16]));
    }
    let entries: Vec<(&str, &[u8])> = names
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let path = std::env::temp_dir().join(format!("nd-open-{name}-{}.ns4b", std::process::id()));
    std::fs::write(&path, stored_archive(&entries).expect("archive")).unwrap();
    path
}

#[test]
fn opening_a_library_of_the_real_shape_takes_tens_of_milliseconds() {
    // 512 programs and 5 000 sound entries: the shape of a full Stage 4 backup's directory
    // (the real file is 3 GB, but only the directory is read here; sounds stay on disk).
    let path = synthetic_library("shape", 512, 5000);
    let t = Instant::now();
    let library = plugin_library(Some(path.clone()));
    let elapsed = t.elapsed();
    let _ = std::fs::remove_file(&path);
    eprintln!("plugin_library on a synthetic 5 512-entry zip: {elapsed:?}");
    assert!(
        library.error_message.is_none(),
        "{:?}",
        library.error_message
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "opening took {elapsed:?}"
    );
}

#[test]
fn no_saved_library_opens_an_empty_tab_at_once() {
    let t = Instant::now();
    let library = plugin_library(None);
    assert!(library.path_input.is_empty() && library.error_message.is_none());
    assert!(t.elapsed() < std::time::Duration::from_millis(200));
}
