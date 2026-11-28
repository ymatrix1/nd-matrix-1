//! The `ns4-library` thread: decodes the requested program and its banks and queues them for
//! `process()`. It stops as soon as the plugin instance closes its [`super::LibraryLink`]
//! (between decodes and inside every wait), so a host that unloads the plugin module never
//! leaves this code running.

use super::archive::{
    bank_key, current_stamp, decode_program, index, read_entry, short_name, wanted_sounds, Indexed,
};
use super::handled::Handled;
use super::{report, QUEUE_WAIT, WAVE_TABLES_WAIT};
use crate::library_select::{label, location};
use crate::library_state::{LibraryPersist, LibraryStatus};
use crate::loader::{LoadJob, LoadSender, Refused};
use crate::shared_sounds::Shared;
use crossbeam_channel::{Receiver, RecvTimeoutError};
use nord_core::piano::NpnoSound;
use nord_core::synth::SynthSampleSound;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Decoded banks shared by every instance of this process (`shared_sounds.rs`), by
/// [`bank_key`].
static PIANOS: Shared<NpnoSound> = Shared::new();
static SAMPLES: Shared<SynthSampleSound> = Shared::new();

/// How often a request for a library that is not there is tried again (an external volume
/// mounted after the host started). ASSUMED: a few seconds is quick enough for a user who just
/// mounted a disk and costs one `stat` per try.
#[cfg(not(test))]
const RETRY_EVERY: Duration = Duration::from_secs(5);
#[cfg(test)]
const RETRY_EVERY: Duration = Duration::from_millis(100);

const CLOSING: &str = "the plugin instance is closing";

pub(super) struct Worker {
    path: PathBuf,
    loads: LoadSender,
    /// Records the newest handled request (`LibraryLink::wait_handled`).
    done: Arc<Handled>,
    /// Set by the link's `Drop`: finish at once.
    closed: Arc<AtomicBool>,
    /// The library, indexed on the first request.
    archive: Option<Arc<Indexed>>,
    /// Banks of the current program, kept alive here (the process-wide `PIANOS` / `SAMPLES`
    /// hold only weak references; a factory piano is hundreds of megabytes).
    pianos: HashMap<String, Arc<NpnoSound>>,
    samples: HashMap<String, Arc<SynthSampleSound>>,
    wave_bank_sent: bool,
    /// The OS update tables were waited for once and did not come: do not wait again.
    wave_wait_failed: bool,
    /// The project's library reference: the path to follow (restored late, or chosen in the
    /// editor) and where the status is reported (D261).
    state: Option<Arc<LibraryPersist>>,
    /// The request to try again because the library file was not there, with its generation.
    retry: Option<(u64, i32)>,
}

impl Worker {
    pub(super) fn new(
        path: PathBuf,
        loads: LoadSender,
        done: Arc<Handled>,
        closed: Arc<AtomicBool>,
        state: Option<Arc<LibraryPersist>>,
    ) -> Self {
        Self {
            state,
            retry: None,
            path,
            loads,
            done,
            closed,
            archive: None,
            pianos: HashMap::new(),
            samples: HashMap::new(),
            wave_bank_sent: false,
            wave_wait_failed: false,
        }
    }

    pub(super) fn run(mut self, rx: Receiver<(u64, i32)>) {
        loop {
            let next = match self.retry {
                Some(again) => match rx.recv_timeout(RETRY_EVERY) {
                    Ok(request) => request,
                    Err(RecvTimeoutError::Timeout) => again,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(request) => request,
                    Err(_) => return,
                },
            };
            let (mut generation, mut want) = next;
            // Only the newest request matters when the host scrubbed through several.
            while let Ok((later_generation, later)) = rx.try_recv() {
                generation = later_generation;
                want = later;
            }
            if self.closing() {
                return;
            }
            self.retry = None;
            self.follow_state();
            if let Some(flat) = location(want) {
                match self.load(flat) {
                    Ok(name) => {
                        self.note_found();
                        report(&format!("program {} loaded: {name}", label(want)));
                    }
                    Err(e) => {
                        report(&format!("program {} not loaded ({e})", label(want)));
                        self.note_missing(generation, want);
                    }
                }
            } else if want == 0 {
                self.unload();
            }
            self.done.record(generation, want);
        }
    }

    /// Switches to the path the project state names now (restored after the worker started, or
    /// chosen in the editor); the old index and banks are let go.
    fn follow_state(&mut self) {
        let Some(path) = self.state.as_ref().and_then(|s| s.path()) else {
            return;
        };
        if path != self.path {
            report(&format!("sound library is now {}", path.display()));
            self.path = path;
            self.archive = None;
            self.pianos.clear();
            self.samples.clear();
        }
    }

    /// A failed load because the file is not there: say so and try again in a few seconds.
    fn note_missing(&mut self, generation: u64, want: i32) {
        if self.path.is_file() {
            return;
        }
        if let Some(s) = &self.state {
            s.set_status(LibraryStatus::Missing(self.path.display().to_string()));
        }
        self.retry = Some((generation, want));
    }

    /// A load worked: a `Missing` status is over.
    fn note_found(&self) {
        if let Some(s) = self
            .state
            .as_ref()
            .filter(|s| matches!(s.status(), LibraryStatus::Missing(_)))
        {
            s.set_status(LibraryStatus::Ready);
        }
    }

    /// Off: the engine goes back to the host parameters (`LoadJob::Unload`); the banks kept here
    /// are released too.
    fn unload(&mut self) {
        self.pianos.clear();
        self.samples.clear();
        match self.push(LoadJob::Unload) {
            Ok(()) => report("program Off: library program released"),
            Err(e) => report(&format!("program Off not applied ({e})")),
        }
    }

    fn closing(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// `Err` once the link is dropped, so a load stops between its stages.
    fn check_open(&self) -> Result<(), String> {
        if self.closing() {
            Err(CLOSING.to_string())
        } else {
            Ok(())
        }
    }

    /// The indexed library. A file replaced on disk since it was indexed (another stamp) is
    /// indexed again, and the banks decoded from the old file are let go: the next request after
    /// `ns4 library set` or a re-export plays the new file without a restart.
    fn open(&mut self) -> Result<Arc<Indexed>, String> {
        if self
            .archive
            .as_ref()
            .is_some_and(|a| a.stamp != current_stamp(&self.path))
        {
            report("the sound library changed on disk: reading it again");
            self.archive = None;
            self.pianos.clear();
            self.samples.clear();
        }
        if self.archive.is_none() {
            self.archive = Some(Arc::new(index(&self.path)?));
        }
        self.archive
            .clone()
            .ok_or_else(|| "library not open".to_string())
    }

    /// Decodes the program at `flat` and its sounds, then queues them. Returns the program name.
    fn load(&mut self, flat: u16) -> Result<String, String> {
        let lib = self.open()?;
        let entry = lib
            .slots
            .iter()
            .find(|(l, _)| *l == flat)
            .map(|(_, n)| n.clone())
            .ok_or_else(|| "no program at that location in the library".to_string())?;
        let prog = decode_program(&self.path, &lib.archive, &entry)?;
        let name = prog.name.clone();
        let (pianos, samples) = wanted_sounds(&lib.archive, &prog);

        let mut jobs: Vec<LoadJob> = Vec::with_capacity(7);
        jobs.push(LoadJob::Program(Box::new(prog)));
        for (slot_b, want) in [(false, &pianos[0]), (true, &pianos[1])] {
            self.check_open()?;
            let sound = match want {
                Some(e) => self.piano(&lib, e),
                None => None,
            };
            jobs.push(LoadJob::Piano { slot_b, sound });
        }
        for (layer, want) in samples.iter().enumerate() {
            self.check_open()?;
            let sound = match want {
                Some(e) => self.sample(&lib, e),
                None => None,
            };
            jobs.push(LoadJob::Sample { layer, sound });
        }
        // Keep only what this program uses (the engine holds its own `Arc`s).
        self.pianos
            .retain(|k, _| pianos.iter().flatten().any(|w| w == k));
        self.samples
            .retain(|k, _| samples.iter().flatten().any(|w| w == k));

        if !self.wave_bank_sent && self.wave_tables_ready() {
            nord_core::synth::prepare_wave_bank();
            jobs.push(LoadJob::WaveBank(true));
            self.wave_bank_sent = true;
        }
        for job in jobs {
            self.push(job)?;
        }
        Ok(name)
    }

    fn piano(&mut self, lib: &Indexed, entry: &str) -> Option<Arc<NpnoSound>> {
        if let Some(s) = self.pianos.get(entry) {
            return Some(Arc::clone(s));
        }
        let key = bank_key(&self.path, &lib.stamp, entry);
        let decoded = PIANOS.get_or_decode(&key, || {
            let mut sound = read_entry(&self.path, &lib.archive, entry)
                .and_then(|b| NpnoSound::from_npno_bytes(&b))?;
            if sound.name.is_empty() {
                sound.name = short_name(entry).to_string();
            }
            Ok(sound)
        });
        match decoded {
            Ok(sound) => {
                self.pianos.insert(entry.to_string(), Arc::clone(&sound));
                Some(sound)
            }
            Err(e) => {
                report(&format!("piano '{entry}' not loaded ({e})"));
                None
            }
        }
    }

    fn sample(&mut self, lib: &Indexed, entry: &str) -> Option<Arc<SynthSampleSound>> {
        if let Some(s) = self.samples.get(entry) {
            return Some(Arc::clone(s));
        }
        let key = bank_key(&self.path, &lib.stamp, entry);
        let decoded = SAMPLES.get_or_decode(&key, || {
            let mut sound = read_entry(&self.path, &lib.archive, entry)
                .and_then(|b| SynthSampleSound::from_nsmp4_bytes(&b).map_err(|e| e.to_string()))?;
            if sound.name.is_empty() {
                sound.name = short_name(entry).to_string();
            }
            Ok(sound)
        });
        match decoded {
            Ok(sound) => {
                self.samples.insert(entry.to_string(), Arc::clone(&sound));
                Some(sound)
            }
            Err(e) => {
                report(&format!("sample bank '{entry}' not loaded ({e})"));
                None
            }
        }
    }

    /// Queues one job, waiting for `process()` to make room (the queue is drained only while
    /// the host runs the plugin). Returns at once when the link is dropped or the queue has no
    /// receiver any more.
    pub(super) fn push(&self, mut job: LoadJob) -> Result<(), String> {
        let deadline = Instant::now() + QUEUE_WAIT;
        loop {
            self.check_open()?;
            match self.loads.push(job) {
                Ok(()) => return Ok(()),
                Err(Refused::Closed) => return Err(CLOSING.to_string()),
                Err(Refused::Full(back)) => {
                    if Instant::now() > deadline {
                        return Err("the plugin is not processing audio".to_string());
                    }
                    job = back;
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    /// Whether the OS update tables are in: waits a little for the install thread
    /// (`wave_tables::install_saved_in_background`) when a file is configured, so the first
    /// program already plays its WAVE sounds from the real tables. Waits only once: a file that
    /// never installs (moved, broken) does not cost every later program change the wait.
    fn wave_tables_ready(&mut self) -> bool {
        if nord_core::synth::wave_tables_installed() {
            return true;
        }
        if self.wave_wait_failed {
            return false;
        }
        let env = std::env::var(crate::wave_tables::WAVE_TABLES_ENV).ok();
        let config = nord_core::user_config::default_config_path();
        if crate::wave_tables::saved_path(env.as_deref(), config.as_deref()).is_none() {
            return false;
        }
        let deadline = Instant::now() + WAVE_TABLES_WAIT;
        while Instant::now() < deadline && !self.closing() {
            if nord_core::synth::wave_tables_installed() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.wave_wait_failed = true;
        report(&format!(
            "OS update tables not installed after {} s: WAVE sounds play from the placeholder \
             tables until a later program load finds them",
            WAVE_TABLES_WAIT.as_secs()
        ));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::load_queue;

    fn worker(loads: LoadSender, closed: Arc<AtomicBool>) -> Worker {
        let done = Arc::new(Handled::default());
        Worker::new(PathBuf::from("unused.ns4b"), loads, done, closed, None)
    }

    #[test]
    fn a_push_returns_at_once_when_the_queue_has_no_receiver_or_the_link_is_closed() {
        let (tx, rx) = load_queue(1);
        let w = worker(tx, Arc::new(AtomicBool::new(false)));
        drop(rx);
        let t = Instant::now();
        assert!(w.push(LoadJob::WaveBank(true)).is_err());
        assert!(t.elapsed() < Duration::from_secs(1), "no 30 s retry loop");

        let (tx, rx) = load_queue(1);
        tx.push(LoadJob::WaveBank(false)).unwrap();
        let closed = Arc::new(AtomicBool::new(true));
        let w = worker(tx, closed);
        let t = Instant::now();
        assert_eq!(w.push(LoadJob::WaveBank(true)), Err(CLOSING.to_string()));
        assert!(t.elapsed() < Duration::from_secs(1));
        assert!(matches!(rx.pop(), Some(LoadJob::WaveBank(false))));
    }

    #[test]
    fn a_failed_wave_table_wait_is_remembered_and_a_closed_worker_loads_nothing() {
        let (tx, _rx) = load_queue(1);
        let mut w = worker(tx, Arc::new(AtomicBool::new(false)));
        w.wave_wait_failed = true;
        let t = Instant::now();
        let ready = w.wave_tables_ready();
        assert!(t.elapsed() < Duration::from_secs(1));
        assert_eq!(ready, nord_core::synth::wave_tables_installed());
        w.closed.store(true, Ordering::SeqCst);
        assert_eq!(w.check_open(), Err(CLOSING.to_string()));
        assert!(w.load(0).is_err());
    }
}
