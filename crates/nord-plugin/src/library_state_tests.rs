//! Host project round trip of the library reference (D262, issue #669): the state blob the
//! host saves is read back into a fresh instance, which reopens the library, tolerates a missing
//! path and keeps the edits the user made over the program.

use crate::auto_library::support::{two_program_backup, write_two_program_backup};
use crate::auto_library::LibrarySource;
use crate::library_off_tests::{block, initialize, program_name};
use crate::library_state::LibraryStatus;
use crate::program_hold::Group;
use crate::NdMatrix1Plugin;
use nih_plug::prelude::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn plugin(source: LibrarySource) -> NdMatrix1Plugin {
    NdMatrix1Plugin {
        library_source: source,
        ..Default::default()
    }
}

/// What the host stores for the project.
fn save(p: &NdMatrix1Plugin) -> std::collections::BTreeMap<String, String> {
    p.params.serialize_fields()
}

fn restore(p: &NdMatrix1Plugin, blob: &std::collections::BTreeMap<String, String>) {
    p.params.deserialize_fields(blob);
}

fn project_with(user: Option<PathBuf>) -> NdMatrix1Plugin {
    plugin(LibrarySource::Project { user })
}

fn saved_project(file: &Path) -> std::collections::BTreeMap<String, String> {
    let mut a = plugin(LibrarySource::Path(file.to_path_buf()));
    initialize(&mut a);
    assert_eq!(program_name(&a).as_deref(), Some("First"));
    save(&a)
}

/// Blocks until the worker's program shows up in the engine (the async path).
fn until_program(p: &mut NdMatrix1Plugin) -> Option<String> {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        block(p);
        if let Some(n) = program_name(p) {
            return Some(n);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

#[test]
fn a_project_reopens_its_library_without_the_users_setting() {
    let file = two_program_backup("proj-path");
    let blob = saved_project(&file);
    assert!(
        blob["library-state"].contains("proj-path"),
        "the path is in the blob"
    );

    let mut b = project_with(None); // no `ns4 library set` on this machine
    restore(&b, &blob);
    initialize(&mut b);
    let _ = std::fs::remove_file(&file);
    assert_eq!(program_name(&b).as_deref(), Some("First"));
    assert_eq!(b.params.library_state.status(), LibraryStatus::Ready);
}

#[test]
fn the_blob_holds_a_path_and_a_hash_and_no_library_bytes() {
    let file = two_program_backup("proj-small");
    let blob = saved_project(&file);
    let _ = std::fs::remove_file(&file);
    let text = &blob["library-state"];
    assert!(text.len() < 400, "{} bytes: {text}", text.len());
    assert!(text.contains("head_hash") && text.contains("size"));
}

#[test]
fn a_state_without_the_new_key_loads_and_uses_the_users_library() {
    let file = two_program_backup("proj-old");
    let mut old = saved_project(&file);
    old.remove("library-state"); // a project saved before D262
    let mut b = project_with(Some(file.clone()));
    restore(&b, &old);
    initialize(&mut b);
    let _ = std::fs::remove_file(&file);
    assert_eq!(program_name(&b).as_deref(), Some("First"));
    assert_eq!(
        b.params.library_state.path(),
        Some(file),
        "recorded for the next save"
    );
}

#[test]
fn a_missing_library_keeps_the_state_says_so_and_loads_when_it_appears() {
    let file = two_program_backup("proj-missing");
    let blob = saved_project(&file);
    let _ = std::fs::remove_file(&file); // the volume is not mounted

    let mut b = project_with(None);
    restore(&b, &blob);
    initialize(&mut b);
    block(&mut b);
    assert_eq!(program_name(&b), None, "the built-in engine plays");
    let msg = b
        .params
        .library_state
        .status()
        .message()
        .expect("a message");
    assert!(msg.starts_with("library not found: "), "{msg}");
    assert!(msg.contains("proj-missing"));
    assert_eq!(
        b.params.library_state.path(),
        Some(file.clone()),
        "state kept"
    );
    assert!(
        save(&b)["library-state"].contains("proj-missing"),
        "and saved again"
    );

    write_two_program_backup(&file, "First"); // mounted later
    let loaded = until_program(&mut b);
    let _ = std::fs::remove_file(&file);
    assert_eq!(loaded.as_deref(), Some("First"));
    assert_eq!(b.params.library_state.status(), LibraryStatus::Ready);
}

#[test]
fn the_edits_armed_over_the_program_survive_the_reload() {
    let file = two_program_backup("proj-edits");
    let mut a = plugin(LibrarySource::Path(file.clone()));
    initialize(&mut a);
    assert!(a.params.program_hold.owned_by_program(Group::Organ));
    // The user moves a drawbar: the host value differs from the one at the load.
    a.params.program_hold.set_test_value("drawbar_1_16", 0.123);
    a.params.program_hold.observe();
    assert!(a.params.program_hold.runs(Group::Organ));
    let blob = save(&a);

    let mut b = project_with(None);
    restore(&b, &blob);
    initialize(&mut b);
    let _ = std::fs::remove_file(&file);
    assert_eq!(program_name(&b).as_deref(), Some("First"));
    assert!(
        b.params.program_hold.runs(Group::Organ),
        "the restored edit wins over the loaded program"
    );
    assert!(
        b.params.program_hold.owned_by_program(Group::Synth),
        "untouched groups stay the program's"
    );
}

#[test]
fn without_edits_the_reloaded_program_owns_every_group() {
    let file = two_program_backup("proj-clean");
    let blob = saved_project(&file);
    let mut b = project_with(None);
    restore(&b, &blob);
    initialize(&mut b);
    let _ = std::fs::remove_file(&file);
    assert!(b.params.program_hold.owned_by_program(Group::Organ));
}
