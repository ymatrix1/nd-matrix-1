//! Tests of the service: the synchronous pump, the worker thread, back
//! pressure, failing jobs, allocation and shutdown. The scratch core has its own tests in
//! `scratch.rs`.
//!
//! OWNER: W1-SVC. Every wait has a deadline, so a defect is a failed test and not a hung run.
//! The worker tests compare with `fixture::probe`, a translator that depends on every word it
//! reads, because the W0 `translate_range` returns no blocks and would make an equality test
//! vacuous (design P10): each such test asserts that the answers are not empty.
use super::fixture::{probe, program, render};
use super::*;
use crate::core::support::{core_with, CODE};
use crate::core::uop::lower::translate_range;
use crate::core::uop::verify::count_alloc::allocs_in;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;
use std::thread::ThreadId;
use std::time::Instant;

/// Longest any test waits for the worker. ASSUMED generous: a loaded CI machine still answers a
/// handful of tiny requests in well under a second.
const DEADLINE: Duration = Duration::from_secs(30);

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < DEADLINE, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

fn req(lo: u32) -> TransReq {
    TransReq {
        lo,
        hi: lo + 4,
        words: vec![0x0001; 4 + TAIL_WORDS].into_boxed_slice(),
    }
}

/// An answer that names its request: one empty block at `lo`.
fn echo(r: TransReq) -> TransDone {
    TransDone {
        blocks: vec![Block::empty(r.lo)],
        uops: Vec::new(),
    }
}

fn echo_service() -> Service {
    Service::spawn_with(|| |r: TransReq| Ok(echo(r))).unwrap()
}

fn pcs(answers: &[TransDone]) -> Vec<u32> {
    answers.iter().map(|d| d.blocks[0].pc).collect()
}

/// Polls until `n` answers arrived.
fn collect(svc: &mut Service, n: usize) -> Vec<TransDone> {
    let mut got = Vec::new();
    wait_until("the answers", || {
        got.extend(std::iter::from_fn(|| svc.poll()));
        got.len() >= n
    });
    got
}

/// Drops `svc` on another thread and fails when that takes longer than [`DEADLINE`].
fn drop_within_deadline(svc: Service) {
    let (done, finished) = std::sync::mpsc::channel();
    thread::spawn(move || {
        drop(svc);
        done.send(()).unwrap();
    });
    finished
        .recv_timeout(DEADLINE)
        .expect("the service dropped in time");
}

#[test]
fn a_synchronous_service_serves_in_order_and_has_no_worker() {
    let mut s = Service::new();
    assert!(s.poll().is_none() && !s.is_threaded() && !s.worker_running());
    for lo in [0x12_0000, 0x12_0100, 0x12_0200] {
        s.submit(req(lo)).unwrap();
    }
    assert_eq!(s.in_flight(), 3);
    assert_eq!(s.pump(echo), 3);
    let answers: Vec<TransDone> = std::iter::from_fn(|| s.poll()).collect();
    assert_eq!(pcs(&answers), [0x12_0000, 0x12_0100, 0x12_0200]);
    assert_eq!((s.in_flight(), s.failed(), s.pump(echo)), (0, 0, 0));
}

#[test]
fn a_full_request_ring_gives_the_request_back() {
    let mut s = Service::new();
    for k in 0..CAP as u32 {
        s.submit(req(k)).unwrap();
    }
    assert_eq!(s.submit(req(99)).unwrap_err().lo, 99);
    assert_eq!(
        s.in_flight(),
        CAP as u64,
        "a refused request is not counted"
    );
    assert_eq!(s.pump(echo), CAP);
}

#[test]
fn pump_stops_at_a_full_answer_ring_and_loses_no_answer() {
    let mut s = Service::new();
    let cap = CAP as u32;
    for lo in 0..cap {
        s.submit(req(lo)).unwrap();
    }
    assert_eq!(s.pump(echo), CAP, "the answer ring is now full");
    for lo in cap..2 * cap {
        s.submit(req(lo)).unwrap();
    }
    assert_eq!(
        s.pump(echo),
        0,
        "no room for an answer: the requests stay queued"
    );
    assert_eq!(s.in_flight(), 2 * u64::from(cap));
    let mut answers = vec![s.poll().unwrap()];
    assert_eq!(s.pump(echo), 1, "exactly the slot the poll freed");
    while answers.len() < 2 * CAP {
        s.pump(echo);
        answers.extend(std::iter::from_fn(|| s.poll()));
    }
    assert_eq!(
        pcs(&answers),
        (0..2 * cap).collect::<Vec<_>>(),
        "in order, none lost"
    );
    assert_eq!((s.in_flight(), s.failed()), (0, 0));
}

#[test]
fn pump_job_turns_an_error_into_an_empty_answer_and_counts_it() {
    let mut s = Service::new();
    for lo in [1, 2, 3] {
        s.submit(req(lo)).unwrap();
    }
    let served = s.pump_job(|r| {
        if r.lo == 2 {
            Err(UopError::Translate("no".into()))
        } else {
            Ok(echo(r))
        }
    });
    let answers: Vec<TransDone> = std::iter::from_fn(|| s.poll()).collect();
    assert_eq!((served, s.failed()), (3, 1));
    let lens: Vec<usize> = answers.iter().map(|d| d.blocks.len()).collect();
    assert_eq!(
        lens,
        [1, 0, 1],
        "the failed request still has its answer, empty"
    );
}

#[test]
fn a_worker_thread_runs_the_job_on_its_own_scratch_core_and_answers_in_order() {
    let mut live = core_with(&program());
    let cfg = UopConfig::default();
    let threads = Arc::new(Mutex::new(Vec::<ThreadId>::new()));
    let seen = Arc::clone(&threads);
    let mut svc = Service::spawn_with(move || {
        let mut scratch = Scratch::new(None);
        let cfg = UopConfig::default();
        move |r: TransReq| {
            seen.lock().unwrap().push(thread::current().id());
            scratch.translate_with(probe, &cfg, &r)
        }
    })
    .unwrap();
    assert!(svc.is_threaded() && svc.worker_running());
    // the second range starts at the first add; the third reuses addresses of the first
    let ranges = [(CODE, CODE + 6), (CODE + 1, CODE + 9), (CODE, CODE + 9)];
    for (lo, hi) in ranges {
        svc.submit(TransReq::capture(&live, lo, hi).unwrap())
            .unwrap();
    }
    assert_eq!(svc.in_flight(), 3);
    let answers = collect(&mut svc, 3);
    for ((lo, hi), got) in ranges.iter().zip(&answers) {
        let direct = probe(&mut live, &cfg, *lo, *hi).unwrap();
        assert!(
            !got.blocks.is_empty(),
            "the translator ran on {lo:#x}..{hi:#x}"
        );
        assert_eq!(render(&got.blocks, &got.uops), render(&direct.0, &direct.1));
    }
    let ids = threads.lock().unwrap();
    assert_eq!(ids.len(), 3);
    assert!(
        ids.iter().all(|id| *id != thread::current().id()),
        "on the worker thread"
    );
    assert_eq!((svc.in_flight(), svc.failed()), (0, 0));
}

#[test]
fn the_default_worker_returns_what_translate_range_returns_for_the_same_words() {
    // With the W0 stub `translate_range` returns no blocks, so this pins the plumbing (request
    // ring, thread, scratch core, answer ring) and, once W1-LOWER lowers real blocks, the
    // equality of the worker contract; the non-vacuous check of the scratch core is the probe
    // test above and in `scratch.rs`.
    let mut live = core_with(&program());
    let cfg = UopConfig::default();
    let mut svc = Service::spawn(cfg.clone(), None).unwrap();
    svc.submit(TransReq::capture(&live, CODE, CODE + 9).unwrap())
        .unwrap();
    let got = collect(&mut svc, 1).remove(0);
    let direct = translate_range(&mut live, &cfg, CODE, CODE + 9).unwrap();
    assert_eq!(render(&got.blocks, &got.uops), render(&direct.0, &direct.1));
    assert_eq!(svc.failed(), 0);
    // an uncovered range is the same error on both sides, answered empty and counted
    let mut bad = TransReq::capture(&live, CODE, CODE + 9).unwrap();
    bad.hi = bad.lo;
    svc.submit(bad).unwrap();
    assert!(collect(&mut svc, 1)[0].blocks.is_empty());
    assert_eq!(svc.failed(), 1);
}

#[test]
fn the_worker_survives_failing_and_panicking_jobs_and_counts_them() {
    let calls = AtomicUsize::new(0);
    let mut svc = Service::spawn_with(move || {
        move |r: TransReq| match calls.fetch_add(1, Ordering::Relaxed) {
            1 => Err(UopError::Translate("refused".into())),
            2 => panic!("a translator bug"),
            _ => Ok(echo(r)),
        }
    })
    .unwrap();
    for lo in 10..14 {
        svc.submit(req(lo)).unwrap();
    }
    let answers = collect(&mut svc, 4);
    let lens: Vec<usize> = answers.iter().map(|d| d.blocks.len()).collect();
    assert_eq!(lens, [1, 0, 0, 1], "one answer per request, in order");
    assert_eq!(answers[3].blocks[0].pc, 13);
    assert_eq!((svc.failed(), svc.worker_running()), (2, true));
}

#[test]
fn a_full_answer_ring_holds_the_worker_and_no_answer_is_lost() {
    let mut svc = echo_service();
    // room for CAP answers, one in the worker's hands and CAP - 1 more queued behind it
    let total = 2 * CAP as u32;
    let start = Instant::now();
    for lo in 0..total {
        let mut r = req(lo);
        while let Err(back) = svc.submit(r) {
            assert!(
                start.elapsed() < DEADLINE,
                "the worker stopped taking requests"
            );
            r = back;
            thread::yield_now();
        }
    }
    // the worker answers CAP requests, then holds the next answer: CAP - 1 stay queued
    wait_until("the worker to hold an answer", || {
        svc.to_worker.len() == CAP - 1
    });
    thread::sleep(Duration::from_millis(20));
    assert_eq!(
        svc.to_worker.len(),
        CAP - 1,
        "blocked, not dropping answers"
    );
    let answers = collect(&mut svc, total as usize);
    assert_eq!(
        pcs(&answers),
        (0..total).collect::<Vec<_>>(),
        "in order, none lost"
    );
    assert_eq!((svc.in_flight(), svc.failed()), (0, 0));
}

#[test]
fn submit_and_poll_allocate_nothing_with_or_without_a_worker() {
    let mut sync = Service::new();
    let r = req(0x100);
    let (res, n) = allocs_in(|| sync.submit(r));
    assert!(res.is_ok());
    assert_eq!(n, 0, "submit moves the request into the ring");
    assert_eq!(sync.pump(echo), 1);
    let (answer, n) = allocs_in(|| sync.poll());
    assert_eq!((answer.map(|d| d.blocks.len()), n), (Some(1), 0));
    assert_eq!(
        allocs_in(|| sync.poll()).1,
        0,
        "an empty poll allocates nothing"
    );

    let mut svc = echo_service();
    let r = req(0x200);
    let (res, n) = allocs_in(|| svc.submit(r));
    assert!(res.is_ok());
    assert_eq!(n, 0, "waking the worker allocates nothing");
    let mut allocated = 0;
    let mut got = None;
    wait_until("the answer", || {
        let (d, n) = allocs_in(|| svc.poll());
        allocated += n;
        got = d;
        got.is_some()
    });
    assert_eq!((got.map(|d| d.blocks[0].pc), allocated), (Some(0x200), 0));
}

#[test]
fn dropping_the_service_stops_an_idle_worker_and_one_that_is_busy() {
    drop_within_deadline(echo_service());
    drop_within_deadline(Service::new());

    let release = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicUsize::new(0));
    let (hold, begun) = (Arc::clone(&release), Arc::clone(&started));
    let mut svc = Service::spawn_with(move || {
        move |r: TransReq| {
            begun.fetch_add(1, Ordering::SeqCst);
            while !hold.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(echo(r))
        }
    })
    .unwrap();
    for lo in 0..3 {
        svc.submit(req(lo)).unwrap();
    }
    wait_until("the first job to start", || {
        started.load(Ordering::SeqCst) >= 1
    });
    // the drop waits for the job it cannot interrupt, then leaves the rest of the queue
    let dropper = thread::spawn(move || drop(svc));
    thread::sleep(Duration::from_millis(20));
    assert!(!dropper.is_finished(), "joined the busy worker");
    release.store(true, Ordering::SeqCst);
    wait_until("the drop", || dropper.is_finished());
    dropper.join().unwrap();
    assert!(started.load(Ordering::SeqCst) >= 1);
}
