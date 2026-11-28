//! Headless program loading from the user's saved `.ns4b` sound library (`library_setting`), for
//! hosts that never open the editor (SoundCraft on Windows has no VST3 editor windows) and for a
//! reloaded project: the `library_program` parameter names a location, a worker thread
//! (`worker.rs`) decodes that program with the piano and sample banks it references
//! (`archive.rs`), and hands them to `process()` through the same load queue the editor uses
//! (`loader.rs`, D218).
//!
//! The audio thread only sends the wanted location (a `Copy` through a bounded channel: no
//! allocation, no lock) and later drains the ready values. Everything that reads or decodes runs
//! on the worker. The WAVE stage is switched on once the OS update tables are installed
//! (`wave_tables.rs`). Dropping the [`LibraryLink`] stops the worker and joins it, so no code of
//! this module runs after the host unloaded the plugin.

mod archive;
mod handled;
#[cfg(test)]
pub(crate) mod support;
mod worker;

use crate::loader::LoadSender;
use crossbeam_channel::{bounded, Sender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Requests the audio thread may queue before the oldest are dropped (only the last one counts).
/// ASSUMED: a host scrubbing an automation lane sends fewer than this per worker turn.
const REQUEST_CAPACITY: usize = 8;
/// How long a load waits for `process()` to drain the queue before it gives up. ASSUMED:
/// longer than a host keeps an instance activated without running it.
pub(crate) const QUEUE_WAIT: Duration = Duration::from_secs(30);
/// How long the first load waits for the OS update tables to finish installing. ASSUMED:
/// above the parse time of the OS update file on a desktop (about 1 s).
pub(crate) const WAVE_TABLES_WAIT: Duration = Duration::from_secs(5);
/// `plugin.log` is emptied when it is larger than this at the next message. ASSUMED: a year of
/// sessions stays well under it.
const LOG_CAP: u64 = 1 << 20;

/// Where an instance finds its library: the user's saved one (the plugin in a host), or a given
/// path or none (tests and tools that host the plugin without wanting a multi-second load).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LibrarySource {
    #[default]
    Saved,
    None,
    Path(PathBuf),
}

impl LibrarySource {
    /// The library path for this source, if any.
    pub fn path(&self) -> Option<PathBuf> {
        match self {
            Self::Saved => saved_library(),
            Self::None => None,
            Self::Path(p) => Some(p.clone()),
        }
    }
}

/// The plugin's end of the worker: sends wanted locations; dropping it stops the worker.
pub struct LibraryLink {
    /// Requests as `(generation, parameter value)`; generations count up from 1.
    tx: Option<Sender<(u64, i32)>>,
    /// Generation of the newest request that was queued (one producer: the audio thread).
    sent: AtomicU64,
    /// The newest request the worker finished with, for [`LibraryLink::wait_handled`].
    handled: Arc<handled::Handled>,
    closed: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LibraryLink {
    /// Starts the worker for the library at `path`, or returns `None` when there is none (or the
    /// thread cannot start). Nothing is read until the first request.
    pub fn start(path: Option<PathBuf>, loads: LoadSender) -> Option<Self> {
        let Some(path) = path else {
            report("no sound library set (ns4 library set <backup.ns4b>)");
            return None;
        };
        let (tx, rx) = bounded(REQUEST_CAPACITY);
        let handled = Arc::new(handled::Handled::default());
        let closed = Arc::new(AtomicBool::new(false));
        let worker = worker::Worker::new(path, loads, Arc::clone(&handled), Arc::clone(&closed));
        let thread = std::thread::Builder::new()
            .name("ns4-library".into())
            .spawn(move || worker.run(rx))
            .ok()?;
        Some(Self {
            tx: Some(tx),
            sent: AtomicU64::new(0),
            handled,
            closed,
            thread: Some(thread),
        })
    }

    /// Blocks until the worker has finished with the newest request queued so far, provided that
    /// one asked for `program` (its jobs are queued, or it was skipped or failed), at most
    /// `timeout`. For an offline render (a bounce), whose first block comes right after
    /// `initialize`: the program must already be in. `false` on timeout, or when a request for
    /// another value is the newest. Never call it from the audio thread.
    pub fn wait_handled(&self, program: i32, timeout: Duration) -> bool {
        let generation = self.sent.load(Ordering::SeqCst);
        self.handled.wait(generation, program, timeout)
    }

    /// Asks for parameter value `program` (0 = Off: the built-in engine state is restored).
    /// Never blocks or allocates; `false` when the request queue is full (the worker is behind;
    /// the next request wins).
    pub fn request(&self, program: i32) -> bool {
        let generation = self.sent.load(Ordering::SeqCst) + 1;
        let queued = self
            .tx
            .as_ref()
            .is_some_and(|tx| tx.try_send((generation, program)).is_ok());
        if queued {
            self.sent.store(generation, Ordering::SeqCst);
        }
        queued
    }
}

impl Drop for LibraryLink {
    /// Stops the worker and waits for it: it leaves any wait at once and stops between decodes,
    /// so the join lasts at most one bank decode. A host that unloads the plugin module after
    /// dropping the instance then runs no code of this module.
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        drop(self.tx.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Where the worker's messages go besides stderr (a DAW shows neither): `plugin.log` next to
/// the saved settings (`%APPDATA%\ns4\plugin.log` on Windows), appended; emptied once it
/// passes [`LOG_CAP`].
pub fn log_path() -> Option<PathBuf> {
    Some(
        nord_core::user_config::default_config_path()?
            .parent()?
            .join("plugin.log"),
    )
}

/// Prints `msg` on stderr and appends it to [`log_path`] (silently when that fails).
pub(crate) fn report(msg: &str) {
    eprintln!("nd matrix 1: {msg}");
    if let Some(p) = log_path() {
        append_capped(&p, msg, LOG_CAP);
    }
}

/// Appends a line to `path`, emptying the file first when it is already larger than `cap`.
fn append_capped(path: &std::path::Path, msg: &str, cap: u64) {
    use std::io::Write;
    let oversized = std::fs::metadata(path).is_ok_and(|m| m.len() > cap);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(!oversized)
        .write(true)
        .truncate(oversized)
        .open(path);
    if let Ok(mut f) = file {
        let _ = writeln!(f, "{msg}");
    }
}

/// The saved library of this user: `NS4_LIBRARY`, else the path `ns4 library set` stored.
pub fn saved_library() -> Option<PathBuf> {
    let env = std::env::var(nord_core::library_setting::LIBRARY_ENV).ok();
    let config = nord_core::user_config::default_config_path();
    nord_core::library_setting::effective_path(env.as_deref(), config.as_deref())
}

#[cfg(test)]
mod tests {
    use super::support::{drain_until, two_program_backup, write_two_program_backup};
    use super::*;
    use crate::loader::{load_queue, LoadJob};
    use std::path::Path;
    use std::time::Instant;

    #[test]
    fn a_request_decodes_the_program_at_that_location_with_empty_sound_slots() {
        let path = two_program_backup("load");
        let (tx, rx) = load_queue(16);
        let link = LibraryLink::start(Some(path.clone()), tx).expect("worker");
        assert!(link.request(2));
        let jobs = drain_until(&rx, 6);
        let _ = std::fs::remove_file(&path);
        assert_eq!(jobs.len(), 6, "program + 2 pianos + 3 samples");
        match &jobs[0] {
            LoadJob::Program(p) => {
                assert_eq!(p.name, "Second");
                assert_eq!(p.organ.drawbars_a[8], 8);
            }
            other => panic!("first job is not the program: {other:?}"),
        }
        assert!(matches!(
            jobs[1],
            LoadJob::Piano {
                slot_b: false,
                sound: None
            }
        ));
        assert!(matches!(
            jobs[2],
            LoadJob::Piano {
                slot_b: true,
                sound: None
            }
        ));
        assert!(matches!(
            jobs[5],
            LoadJob::Sample {
                layer: 2,
                sound: None
            }
        ));
    }

    #[test]
    fn wait_handled_returns_once_the_jobs_are_queued_and_times_out_otherwise() {
        let path = two_program_backup("wait");
        let (tx, rx) = load_queue(16);
        let link = LibraryLink::start(Some(path.clone()), tx).expect("worker");
        assert!(
            !link.wait_handled(1, Duration::from_millis(50)),
            "nothing requested"
        );
        assert!(link.request(1));
        assert!(link.wait_handled(1, Duration::from_secs(10)));
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            rx.pop().map(|j| matches!(j, LoadJob::Program(_))),
            Some(true)
        );
        // Off is handled too (nothing queued), so an offline render never waits on it.
        assert!(link.request(0));
        assert!(link.wait_handled(0, Duration::from_secs(10)));
    }

    #[test]
    fn many_program_changes_do_not_make_a_later_wait_miss_its_signal() {
        let path = two_program_backup("stale");
        let (tx, rx) = load_queue(1024);
        let link = LibraryLink::start(Some(path.clone()), tx).expect("worker");
        // The reviewer's probe: ten changes nobody waited for, then the one a host waits on.
        assert!(link.request(1));
        assert!(link.wait_handled(1, Duration::from_secs(30)), "first load");
        for _ in 0..9 {
            assert!(link.request(1));
            // Long enough for the worker to handle each one on its own (cached after the first).
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(link.request(2));
        // The worker finishes before anyone waits (a host calls `initialize` later).
        std::thread::sleep(Duration::from_millis(1500));
        let t = Instant::now();
        assert!(link.wait_handled(2, Duration::from_secs(3)));
        assert!(t.elapsed() < Duration::from_secs(3));
        let _ = std::fs::remove_file(&path);
        let jobs = drain_until(&rx, 1);
        assert!(!jobs.is_empty());
    }

    #[test]
    fn a_stale_equal_value_does_not_satisfy_a_newer_request() {
        let path = two_program_backup("stale-eq");
        let (tx, _rx) = load_queue(64);
        let link = LibraryLink::start(Some(path.clone()), tx).expect("worker");
        assert!(link.request(1));
        assert!(link.wait_handled(1, Duration::from_secs(10)));
        // A second request for program 1 is queued but may not be handled yet: a wait for it
        // must not return on the first one's record unless the worker really got to the second.
        assert!(link.request(2));
        assert!(link.request(1));
        assert!(link.wait_handled(1, Duration::from_secs(10)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn off_unloads_missing_locations_queue_nothing_and_the_newest_request_wins() {
        let path = two_program_backup("skip");
        let (tx, rx) = load_queue(16);
        let link = LibraryLink::start(Some(path.clone()), tx).expect("worker");
        // Off queues one `Unload` (the plugin restores the built-in engine state).
        assert!(link.request(0));
        assert!(link.wait_handled(0, Duration::from_secs(10)));
        assert!(matches!(rx.pop(), Some(LoadJob::Unload)));
        assert!(rx.is_empty());
        // An empty location loads nothing.
        assert!(link.request(300));
        assert!(link.wait_handled(300, Duration::from_secs(10)));
        assert!(rx.is_empty(), "an empty location loads nothing");
        assert!(link.request(2));
        assert!(link.request(1));
        assert!(link.wait_handled(1, Duration::from_secs(10)));
        let _ = std::fs::remove_file(&path);
        let mut jobs = Vec::new();
        while let Some(j) = rx.pop() {
            jobs.push(j);
        }
        // The worker may have served both or only the newest; either way the last is "First".
        let names: Vec<String> = jobs
            .iter()
            .filter_map(|j| match j {
                LoadJob::Program(p) => Some(p.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names.last().map(String::as_str), Some("First"));
    }

    #[test]
    fn a_library_replaced_on_disk_is_read_again_on_the_next_request() {
        let path = two_program_backup("replaced");
        let (tx, rx) = load_queue(64);
        let link = LibraryLink::start(Some(path.clone()), tx).expect("worker");
        assert!(link.request(1));
        assert!(link.wait_handled(1, Duration::from_secs(30)));
        let all = |rx: &crate::loader::LoadReceiver| {
            let mut jobs = Vec::new();
            while let Some(j) = rx.pop() {
                jobs.push(j);
            }
            jobs
        };
        let program = |jobs: &[LoadJob]| {
            jobs.iter().find_map(|j| match j {
                LoadJob::Program(p) => Some(p.name.clone()),
                _ => None,
            })
        };
        assert_eq!(program(&all(&rx)).as_deref(), Some("First"));
        // `ns4 library set` / a re-export: another file (another length) at the same path.
        write_two_program_backup(&path, "FirstNew");
        assert!(link.request(1));
        assert!(link.wait_handled(1, Duration::from_secs(30)));
        let second = program(&all(&rx));
        let _ = std::fs::remove_file(&path);
        assert_eq!(second.as_deref(), Some("FirstNew"));
    }

    #[test]
    fn no_library_means_no_worker_and_a_bad_file_is_reported_not_fatal() {
        let (tx, rx) = load_queue(4);
        assert!(LibraryLink::start(None, tx.clone()).is_none());
        let bad = std::env::temp_dir().join(format!("nd-autolib-bad-{}.ns4b", std::process::id()));
        std::fs::write(&bad, b"nope").unwrap();
        let link = LibraryLink::start(Some(bad.clone()), tx).expect("worker");
        assert!(link.request(1));
        std::thread::sleep(Duration::from_millis(300));
        let _ = std::fs::remove_file(&bad);
        assert!(rx.is_empty());
    }

    #[test]
    fn dropping_the_link_stops_the_worker_even_with_a_full_load_queue() {
        let path = two_program_backup("drop");
        // A queue of one that nobody drains: the worker would retry for QUEUE_WAIT.
        let (tx, _rx) = load_queue(1);
        let link = LibraryLink::start(Some(path.clone()), tx).expect("worker");
        assert!(link.request(1));
        std::thread::sleep(Duration::from_millis(200));
        let t = Instant::now();
        drop(link);
        let _ = std::fs::remove_file(&path);
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "drop joined the worker in {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn the_library_source_resolves_to_a_path_or_nothing() {
        assert_eq!(LibrarySource::None.path(), None);
        let p = PathBuf::from("x.ns4b");
        assert_eq!(LibrarySource::Path(p.clone()).path(), Some(p));
        assert_eq!(LibrarySource::default(), LibrarySource::Saved);
    }

    #[test]
    fn the_log_sits_next_to_the_settings_and_is_emptied_past_the_cap() {
        if let Some(p) = log_path() {
            assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("plugin.log"));
            assert_eq!(
                p.parent(),
                nord_core::user_config::default_config_path()
                    .as_deref()
                    .and_then(Path::parent)
            );
        }
        let log = std::env::temp_dir().join(format!("nd-autolib-log-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&log);
        append_capped(&log, "one", 16);
        append_capped(&log, "two", 16);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "one\ntwo\n");
        append_capped(&log, "three", 6);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "three\n");
        let _ = std::fs::remove_file(&log);
    }
}
