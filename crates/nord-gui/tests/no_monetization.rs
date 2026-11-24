//! Guard for `ns4-s3pn`: the shipped GUI has no monetization UI (sponsor, donate, funding links).
//! The project stays strictly non-commercial under PolyForm Noncommercial 1.0.0
//! (`docs/LEGAL.md`). This test fails if a source file under `crates/nord-gui/src` names a
//! funding page or a sponsor / donate control.

use std::fs;
use std::path::Path;

/// Words that mark a monetization control or link. Built at run time so this file does not
/// match itself (it lives in `tests/`, outside the scanned `src/`).
fn banned() -> Vec<String> {
    [
        ["spon", "sor"],
        ["dona", "te"],
        ["pay", "pal"],
        ["ko-", "fi"],
        ["patr", "eon"],
    ]
    .iter()
    .map(|parts| parts.concat())
    .collect()
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn gui_sources_carry_no_monetization_ui() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(
        !files.is_empty(),
        "no sources found under {}",
        src.display()
    );
    let words = banned();
    let mut hits = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file)
            .expect("read source")
            .to_lowercase();
        for w in &words {
            if text.contains(w.as_str()) {
                hits.push(format!("{} mentions `{w}`", file.display()));
            }
        }
    }
    assert!(hits.is_empty(), "monetization UI found: {hits:#?}");
}

#[test]
fn banned_word_list_is_not_empty() {
    assert!(banned().len() >= 5);
}
