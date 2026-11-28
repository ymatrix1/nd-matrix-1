//! The translation service: requests in, translated blocks out, through two rings.
//!
//! OWNER: W1-SVC (thread, request ring, scratch core); the native validation of what comes back
//! is W4-NSVC. A request carries the code words of a range so the worker never touches the
//! engine's `Core`; the answer carries blocks whose `first` indexes are relative to its own
//! `uops` (the contract of `lower::translate_range`), installed by `UopHot::install` at a pass
//! boundary after the installer has compared each `Block::code_hash` with the live core (the
//! words were copied at request time and may be stale).
//!
//! Two modes share the same two rings:
//!
//! * [`Service::new`]: synchronous. [`Service::pump`] (or [`Service::pump_job`]) plays the
//!   worker on the calling thread; this is `UopConfig::async_translate == false`.
//! * [`Service::spawn`] and [`Service::spawn_with`]: one worker thread that owns the worker ends
//!   of the rings and a [`Scratch`] core. The audio side only calls [`Service::submit`] and
//!   [`Service::poll`]: no lock, no allocation (a request owns its words, an answer its vectors,
//!   both move through the ring); `submit` also wakes the worker with `Thread::unpark`, a
//!   futex-style signal that neither blocks nor allocates.
//!
//! Contract: one answer per accepted request, in request order. A translation that fails (an
//! `Err` or a panic of the job) is answered with an empty [`TransDone`] and counted by
//! [`Service::failed`]; the range then simply stays with the interpreter. The worker never drops
//! an answer: when the answer ring is full it waits for the audio side to `poll`. Dropping the
//! service stops the worker after the translation it is running and joins it; requests still
//! queued are dropped.
//!
//! `stop_pc` is fixed at spawn. A stop pc learnt later is repaired by the driver (design P7:
//! `uop_begin_pass` splits a block that holds it), so it is not a message to the worker.
pub mod scratch;

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod service_tests;

use super::ring::{Receiver, Ring, Sender};
use crate::core::uop::config::{UopConfig, UopError};
use crate::core::uop::run::CODE_SPAN;
use crate::core::uop::types::{Block, Uop};
use crate::core::Core;
pub use scratch::Scratch;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Slots of each ring. ASSUMED: a handful of ranges are in flight at once (program changes).
pub const CAP: usize = 16;

/// Parcels copied after the end of a range. ASSUMED: the last instruction of a range starts
/// inside it and may be three parcels long (two past `hi`), and a delayed branch there owes two
/// more instructions of up to three parcels, so eight parcels past `hi` cover the longest read a
/// lowering can need (the instruction length of the VISA classes, `crate::visa::classify`, is at
/// most three). What would confirm it: W1-LOWER's translator never reads a parcel past
/// `hi + TAIL_WORDS`, which W3-FUZZ checks by translating on the live core and on a scratch core
/// holding only the captured words.
pub const TAIL_WORDS: usize = 8;

/// How long an idle worker sleeps before it looks at the ring again. ASSUMED: a `submit` wakes it
/// at once, so this only bounds a missed wake-up and the time to notice a stop.
const IDLE_PARK: Duration = Duration::from_millis(100);

/// How long a worker holding an answer waits for room in a full answer ring. ASSUMED: `poll`
/// does not wake it, translation answers are rare, a millisecond is invisible next to them.
const FULL_PARK: Duration = Duration::from_millis(1);

/// A request to translate the short words `[lo, hi)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransReq {
    pub lo: u32,
    pub hi: u32,
    /// The code words `lo..hi` plus the tail of the last instruction ([`TAIL_WORDS`]), copied at
    /// request time.
    pub words: Box<[u16]>,
}

impl TransReq {
    /// Copies the parcels of `lo..hi` and the [`TAIL_WORDS`] after them out of `core` (one
    /// allocation). `None` when the range is empty, longer than the short-word window the engine
    /// can hold ([`CODE_SPAN`], so a bad range cannot ask for gigabytes) or lies in ISA-mode code
    /// (`Core::isa_at`): its 48-bit words do not travel as parcels, so that range is translated
    /// on the core itself.
    pub fn capture(core: &Core, lo: u32, hi: u32) -> Option<TransReq> {
        if lo >= hi || hi - lo > CODE_SPAN || core.isa_at(lo) {
            return None;
        }
        let n = (hi - lo) as usize + TAIL_WORDS;
        let words = (0..n)
            .map(|k| core.parcel_at(lo.wrapping_add(k as u32)))
            .collect();
        Some(TransReq { lo, hi, words })
    }
}

/// The answer to one request.
#[derive(Clone, Debug, Default)]
pub struct TransDone {
    pub blocks: Vec<Block>,
    pub uops: Vec<Uop>,
}

/// The worker thread and the flag that stops it.
struct Worker {
    handle: JoinHandle<()>,
    stop: Arc<AtomicBool>,
}

/// The request and answer rings of one engine, and the worker thread when there is one.
pub struct Service {
    to_worker: Sender<TransReq, CAP>,
    from_worker: Receiver<TransDone, CAP>,
    /// The worker ends of the rings: kept here for [`Service::pump`] until a thread takes them.
    local: Option<(Receiver<TransReq, CAP>, Sender<TransDone, CAP>)>,
    worker: Option<Worker>,
    failed: Arc<AtomicU64>,
    submitted: u64,
    polled: u64,
}

impl Default for Service {
    fn default() -> Self {
        Service::new()
    }
}

impl Service {
    /// An idle synchronous service: no thread, [`Service::pump`] does the work.
    pub fn new() -> Self {
        let (to_worker, req_rx) = Ring::channel();
        let (done_tx, from_worker) = Ring::channel();
        Service {
            to_worker,
            from_worker,
            local: Some((req_rx, done_tx)),
            worker: None,
            failed: Arc::new(AtomicU64::new(0)),
            submitted: 0,
            polled: 0,
        }
    }

    /// A service with one worker thread that translates with `lower::translate_range` on its own
    /// [`Scratch`] core: the request words are what the translator sees, so the answer is what
    /// `translate_range` returns for the same words.
    pub fn spawn(cfg: UopConfig, stop_pc: Option<u32>) -> std::io::Result<Self> {
        Service::spawn_with(move || {
            let mut scratch = Scratch::new(stop_pc);
            move |req: TransReq| scratch.translate(&cfg, &req)
        })
    }

    /// A service whose worker thread runs the job `init` builds. `init` runs on the worker thread
    /// (so the job may own a `Core`, which need not be `Send`); the job answers one request at a
    /// time. The native service uses this with its own job (emit, validate).
    pub fn spawn_with<J>(init: impl FnOnce() -> J + Send + 'static) -> std::io::Result<Self>
    where
        J: FnMut(TransReq) -> Result<TransDone, UopError>,
    {
        let mut svc = Service::new();
        let (rx, tx) = svc
            .local
            .take()
            .expect("a new service holds the worker ends");
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, failed) = (Arc::clone(&stop), Arc::clone(&svc.failed));
        let handle = thread::Builder::new()
            .name("uop-translate".into())
            .spawn(move || serve(init(), rx, tx, &flag, &failed))?;
        svc.worker = Some(Worker { handle, stop });
        Ok(svc)
    }

    /// Whether a worker thread serves this service.
    pub fn is_threaded(&self) -> bool {
        self.worker.is_some()
    }

    /// Whether the worker thread is alive (false for a synchronous service and after the worker
    /// died, which only a failing `init` can cause: a failing job is caught).
    pub fn worker_running(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|w| !w.handle.is_finished())
    }

    /// Queues a request, or gives it back when [`CAP`] are already waiting. Never blocks and
    /// never allocates.
    pub fn submit(&mut self, req: TransReq) -> Result<(), TransReq> {
        self.to_worker.push(req)?;
        self.submitted += 1;
        if let Some(w) = &self.worker {
            w.handle.thread().unpark();
        }
        Ok(())
    }

    /// Takes the oldest finished translation. Never blocks and never allocates.
    pub fn poll(&mut self) -> Option<TransDone> {
        let done = self.from_worker.pop()?;
        self.polled += 1;
        Some(done)
    }

    /// Requests accepted by [`Service::submit`] whose answer [`Service::poll`] has not returned.
    pub fn in_flight(&self) -> u64 {
        self.submitted - self.polled
    }

    /// Translations that failed (an error or a panic of the job), answered with an empty
    /// [`TransDone`].
    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }

    /// Translates every queued request with `translate` on the calling thread and queues the
    /// answers. It stops when the answer ring is full and leaves the remaining requests queued,
    /// so no answer is ever lost. Returns the number of requests served; 0 for a threaded
    /// service, whose worker owns the other ends of the rings.
    pub fn pump(&mut self, mut translate: impl FnMut(TransReq) -> TransDone) -> usize {
        self.pump_job(|req| Ok(translate(req)))
    }

    /// [`Service::pump`] with a job that can fail; a failure is answered with an empty
    /// [`TransDone`] and counted like the worker thread counts it.
    pub fn pump_job(
        &mut self,
        mut job: impl FnMut(TransReq) -> Result<TransDone, UopError>,
    ) -> usize {
        let Some((rx, tx)) = self.local.as_mut() else {
            return 0;
        };
        let mut served = 0;
        while !tx.is_full() {
            let Some(req) = rx.pop() else { break };
            let answer = job(req).unwrap_or_else(|_| {
                self.failed.fetch_add(1, Ordering::Relaxed);
                TransDone::default()
            });
            // room was checked above and only this call fills the ring
            if tx.push(answer).is_ok() {
                served += 1;
            }
        }
        served
    }
}

impl Drop for Service {
    /// Stops the worker after the translation it is running and joins it.
    fn drop(&mut self) {
        if let Some(w) = self.worker.take() {
            w.stop.store(true, Ordering::Release);
            w.handle.thread().unpark();
            let _ = w.handle.join();
        }
    }
}

/// The worker loop: pop a request, run the job, push the answer, wait when idle or blocked.
fn serve<J>(
    mut job: J,
    mut rx: Receiver<TransReq, CAP>,
    mut tx: Sender<TransDone, CAP>,
    stop: &AtomicBool,
    failed: &AtomicU64,
) where
    J: FnMut(TransReq) -> Result<TransDone, UopError>,
{
    while !stop.load(Ordering::Acquire) {
        let Some(req) = rx.pop() else {
            thread::park_timeout(IDLE_PARK);
            continue;
        };
        let mut answer = match catch_unwind(AssertUnwindSafe(|| job(req))) {
            Ok(Ok(done)) => done,
            Ok(Err(_)) | Err(_) => {
                failed.fetch_add(1, Ordering::Relaxed);
                TransDone::default()
            }
        };
        while let Err(back) = tx.push(answer) {
            if stop.load(Ordering::Acquire) {
                return;
            }
            answer = back;
            thread::park_timeout(FULL_PARK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::program;
    use super::*;
    use crate::core::support::{core_with, CODE};

    #[test]
    fn capture_copies_the_range_and_its_tail_and_declines_what_cannot_travel() {
        let live = core_with(&program());
        let r = TransReq::capture(&live, CODE, CODE + 4).unwrap();
        assert_eq!(
            (r.lo, r.hi, r.words.len()),
            (CODE, CODE + 4, 4 + TAIL_WORDS)
        );
        assert!(r
            .words
            .iter()
            .enumerate()
            .all(|(k, w)| *w == live.parcel_at(CODE + k as u32)));
        assert_eq!(
            &r.words[..9],
            &program()[..],
            "the whole program is inside range plus tail"
        );
        assert!(TransReq::capture(&live, CODE, CODE).is_none(), "empty");
        assert!(
            TransReq::capture(&live, CODE + 1, CODE).is_none(),
            "inverted"
        );
        assert!(
            TransReq::capture(&live, CODE, CODE + CODE_SPAN + 1).is_none(),
            "wider than the window"
        );
        assert!(TransReq::capture(&live, 0, CODE_SPAN + 1).is_none());
        // ISA-mode words are 48 bits: a range below the short-word space stays on the core
        let mut isa = core_with(&program());
        isa.isa_code = vec![0; 3];
        assert!(TransReq::capture(&isa, 0x100, 0x110).is_none());
        assert!(TransReq::capture(&isa, CODE, CODE + 4).is_some());
        // the same low range is plain VISA code on a core without an ISA image
        assert!(TransReq::capture(&live, 0x100, 0x110).is_some());
    }

    #[test]
    fn the_service_can_move_to_another_thread() {
        fn is_send<T: Send>() {}
        is_send::<Service>();
        is_send::<TransReq>();
        is_send::<TransDone>();
    }
}
