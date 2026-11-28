//! The worker follows the path of the project state (D262): a path restored or chosen after it
//! started is used for the next request, and a library that is not there is tried again.

use super::support::{drain_until, two_program_backup, write_two_program_backup};
use super::LibraryLink;
use crate::library_state::{LibraryPersist, LibraryStatus};
use crate::loader::{load_queue, LoadJob};
use std::sync::Arc;
use std::time::Duration;

fn state() -> Arc<LibraryPersist> {
    Arc::new(LibraryPersist::new(Arc::default(), Arc::default()))
}

fn program_name(jobs: &[LoadJob]) -> Option<String> {
    jobs.iter().find_map(|j| match j {
        LoadJob::Program(p) => Some(p.name.clone()),
        _ => None,
    })
}

#[test]
fn a_path_chosen_after_the_start_is_used_by_the_next_request() {
    let first = two_program_backup("follow-a");
    let second = two_program_backup("follow-b");
    write_two_program_backup(&second, "Renamed");
    let st = state();
    let (tx, rx) = load_queue(16);
    let link = LibraryLink::start_with(Some(first.clone()), tx, Some(st.clone())).expect("worker");
    st.remember(&second);
    assert!(link.request(1));
    let jobs = drain_until(&rx, 6);
    for f in [&first, &second] {
        let _ = std::fs::remove_file(f);
    }
    assert_eq!(program_name(&jobs).as_deref(), Some("Renamed"));
}

#[test]
fn a_missing_library_is_reported_and_loaded_once_the_file_appears() {
    let path = two_program_backup("follow-missing");
    let _ = std::fs::remove_file(&path);
    let st = state();
    st.remember(&path);
    let (tx, rx) = load_queue(16);
    let link = LibraryLink::start_with(Some(path.clone()), tx, Some(st.clone())).expect("worker");
    assert!(link.request(1));
    assert!(link.wait_handled(1, Duration::from_secs(10)));
    assert_eq!(
        st.status(),
        LibraryStatus::Missing(path.display().to_string())
    );
    assert!(rx.pop().is_none(), "nothing to load yet");
    write_two_program_backup(&path, "First"); // the volume is mounted
    let jobs = drain_until(&rx, 6);
    let _ = std::fs::remove_file(&path);
    assert_eq!(program_name(&jobs).as_deref(), Some("First"));
    assert_eq!(st.status(), LibraryStatus::Ready);
}
