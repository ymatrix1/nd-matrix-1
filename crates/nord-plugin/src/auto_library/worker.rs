//! The `ns4-library` thread: decodes the requested program and its banks and queues them for
//! `process()`. It stops as soon as the plugin instance closes its [`super::LibraryLink`]
//! (between decodes and inside every wait), so a host that unloads the plugin module never
//! leaves this code running.

use super::archive::{
    bank_key, decode_program, index, read_entry, short_name, wanted_sounds, Indexed,
};
use super::handled::Handled;
use super::{report, QUEUE_WAIT, WAVE_TABLES_WAIT};
use crate::library_select::{label, location};
use crate::loader::{LoadJob, LoadSender, Refused};
use crate::shared_sounds::Shared;
use crossbeam_channel::Receiver;
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
}

impl Worker {
    pub(super) fn new(
        path: PathBuf,
        loads: LoadSender,
        done: Arc<Handled>,
        closed: Arc<AtomicBool>,
    ) -> Self {
        Self {
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
        while let Ok((mut generation, mut want)) = rx.recv() {
            // Only the newest request matters when the host scrubbed through several.
            while let Ok((later_generation, later)) = rx.try_recv() {
                generation = later_generation;
                want = later;
            }
            if self.closing() {
                return;
            }
            if let Some(flat) = location(want) {
                match self.load(flat) {
                    Ok(name) => report(&format!("program {} loaded: {name}", label(want))),
                    Err(e) => report(&format!("program {} not loaded ({e})", label(want))),
                }
            } else if want == 0 {
                self.unload();
            }
            self.done.record(generation, want);
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

    fn open(&mut self) -> Result<Arc<Indexed>, String> {
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
        Worker::new(PathBuf::from("unused.ns4b"), loads, done, closed)
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
