//! The threaded loop of the pass scheduler: one OS thread per [`PassJob`], a spin barrier per tick.
//!
//! OWNER: W1-RUN. The coordinator publishes tick `t` in `epoch`; each worker spins on it, runs its
//! pass and counts in on `done`; the coordinator spins until all workers have counted in. The
//! workers never block on a lock or a channel.
//!
//! A worker that panics (a `UopChecked` difference, an engine bug) never counts in, so a bare
//! barrier would spin for ever. Every worker therefore runs under `catch_unwind` and raises one
//! shared `abort` flag from its handler; the coordinator and every waiting worker poll it in their
//! spin loops and leave, the threads are joined, and the run returns an error naming the core and
//! the pass (the benchmark exits non-zero with that message). The default panic hook still prints
//! the panic location on stderr.
use super::{CoreSeries, PassJob, SchedReport};
use crate::cpu_clock::ThreadClock;
use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// How a worker runs the pass of tick `tick` on its job. The scheduler uses [`PassJob::run_pass`];
/// [`run_ticks_with`] takes another function so a test can make a core fail at a chosen pass.
pub type PassFn = fn(&mut PassJob, u32) -> (u64, bool);

/// Spins before a waiting thread starts to yield its host core.
const SPINS: u32 = 1 << 16;

/// What the threads share: the published tick, the count of workers done with it and the abort
/// flag.
struct Shared {
    epoch: AtomicU64,
    done: AtomicU32,
    abort: AtomicBool,
}

impl Shared {
    /// Spins (then yields) until `ready` or the run is aborted; false when aborted.
    fn wait(&self, ready: impl Fn(&Shared) -> bool) -> bool {
        let mut spins = 0u32;
        while !ready(self) {
            if self.abort.load(Ordering::Acquire) {
                return false;
            }
            spins += 1;
            if spins < SPINS {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
        true
    }
}

/// What a worker thread ended with.
enum Outcome {
    /// Every tick ran: the series and the job back (boxed: a core is a few KB, the other
    /// variants a few words).
    Done(Box<(CoreSeries, PassJob)>),
    /// The core panicked in this tick, with this message.
    Panicked {
        name: String,
        tick: u64,
        why: String,
    },
    /// Another core panicked first; this one stopped waiting.
    Aborted,
}

/// The text of a panic payload.
fn panic_text(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "a panic without a text payload".to_string()
    }
}

/// One worker: runs `total` ticks of `job`; false when the run was aborted before the end.
/// `at` always holds the tick being run, so a panic can say where it happened.
fn work(
    job: &mut PassJob,
    series: &mut CoreSeries,
    at: &mut u64,
    run: PassFn,
    warmup: u32,
    total: u64,
    shared: &Shared,
) -> bool {
    let mut next_msg = 0;
    let mut warm = None;
    job.msgs.sort_by_key(|m| m.pass);
    for tick in 0..total {
        *at = tick;
        if !shared.wait(|s| s.epoch.load(Ordering::Acquire) > tick) {
            return false;
        }
        if tick == u64::from(warmup) {
            // the counters the warm-up passes left: the report subtracts them
            warm = Some(job.core.uop_stats());
        }
        let p = tick as u32;
        if job.before_pass(p, &mut next_msg).is_err() {
            series.all_idle = false;
        }
        // both clocks: wall (the budget claim) and thread CPU (not inflated by a descheduled
        // thread); the barrier tick of the coordinator stays wall time
        let t0 = ThreadClock::start();
        let (steps, idle) = run(job, p);
        let cpu = t0.elapsed_ns();
        series.steps.push(steps);
        series.ns.push(t0.wall_ns());
        series.cpu_ns.push(cpu);
        series.all_idle &= idle;
        shared.done.fetch_add(1, Ordering::AcqRel);
    }
    // no measured pass at all (`ticks == 0`): everything that ran was warm-up
    series.uop_at_warmup = warm.unwrap_or_else(|| job.core.uop_stats());
    true
}

/// Runs `warmup + ticks` ticks of all `jobs` concurrently, one thread per job. Each pass is
/// [`PassJob::run_pass`]. An error when a core panicked, naming it and its pass.
pub fn run_ticks(jobs: Vec<PassJob>, warmup: u32, ticks: u32) -> Result<SchedReport, String> {
    run_ticks_with(jobs, warmup, ticks, |job, _| job.run_pass())
}

/// [`run_ticks`] with `run` as the pass of each tick.
pub fn run_ticks_with(
    jobs: Vec<PassJob>,
    warmup: u32,
    ticks: u32,
    run: PassFn,
) -> Result<SchedReport, String> {
    let total = u64::from(warmup) + u64::from(ticks);
    let n = jobs.len() as u32;
    // calibrate the thread CPU clock now (200 ms on the first call), not inside the first tick
    // where it would stall every core at the barrier
    let _ = crate::cpu_clock::ns_per_tick();
    let shared = Arc::new(Shared {
        epoch: AtomicU64::new(0),
        done: AtomicU32::new(0),
        abort: AtomicBool::new(false),
    });
    let mut handles = Vec::new();
    for mut job in jobs {
        let shared = shared.clone();
        handles.push(std::thread::spawn(move || {
            let mut series = CoreSeries {
                name: job.name.clone(),
                all_idle: true,
                ..CoreSeries::default()
            };
            let mut at = 0;
            let ran = catch_unwind(AssertUnwindSafe(|| {
                work(&mut job, &mut series, &mut at, run, warmup, total, &shared)
            }));
            match ran {
                Ok(true) => Outcome::Done(Box::new((series, job))),
                Ok(false) => Outcome::Aborted,
                Err(payload) => {
                    // the barrier must not wait for a count this core will never make
                    shared.abort.store(true, Ordering::Release);
                    Outcome::Panicked {
                        name: series.name,
                        tick: at,
                        why: panic_text(payload.as_ref()),
                    }
                }
            }
        }));
    }
    let mut tick_ns = Vec::with_capacity(total as usize);
    for tick in 0..total {
        shared.done.store(0, Ordering::Release);
        let t0 = Instant::now();
        shared.epoch.store(tick + 1, Ordering::Release);
        if !shared.wait(|s| s.done.load(Ordering::Acquire) >= n) {
            break;
        }
        tick_ns.push(t0.elapsed().as_nanos() as u64);
    }
    let (mut cores, mut done_jobs, mut failure) = (Vec::new(), Vec::new(), None);
    for h in handles {
        match h.join() {
            Ok(Outcome::Done(done)) => {
                let (s, j) = *done;
                cores.push(s);
                done_jobs.push(j);
            }
            Ok(Outcome::Panicked { name, tick, why }) => {
                failure.get_or_insert(format!("core {name} panicked in pass {tick}: {why}"));
            }
            Ok(Outcome::Aborted) => {}
            Err(payload) => {
                failure.get_or_insert(format!(
                    "a core thread panicked outside its pass: {}",
                    panic_text(payload.as_ref())
                ));
            }
        }
    }
    if let Some(why) = failure {
        return Err(why);
    }
    if cores.len() != n as usize {
        return Err("the run was aborted before every core finished".into());
    }
    Ok(SchedReport {
        cores,
        tick_ns,
        jobs: done_jobs,
    })
}

#[cfg(test)]
mod tests {
    use super::super::support::poll_job;
    use super::super::Engine;
    use super::*;
    use crate::runner::fx_run;
    use crate::runner::live_presets::boot_core;
    use crate::runner::periph_run::PeriphOpts;
    use crate::runner::uop_pass::arm_for;
    use crate::runner::uop_report::measure::{synthetic_image, synthetic_preset, SYNTH_PASS_STEPS};
    use std::time::Duration;

    /// The run on a helper thread with a deadline: a regression that hangs the barrier fails this
    /// test with a message instead of hanging the test run.
    fn bounded(
        jobs: Vec<PassJob>,
        warmup: u32,
        ticks: u32,
        run: PassFn,
    ) -> Result<SchedReport, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run_ticks_with(jobs, warmup, ticks, run));
        });
        rx.recv_timeout(Duration::from_secs(120))
            .expect("the scheduler hung after a panic in a core")
    }

    fn boom_when(job: &mut PassJob, tick: u32, name: &str, at: u32) -> (u64, bool) {
        if job.name == name && tick == at {
            panic!("boom in {name}");
        }
        job.run_pass()
    }

    fn boom_b_at_3(job: &mut PassJob, tick: u32) -> (u64, bool) {
        boom_when(job, tick, "b", 3)
    }

    fn boom_a_at_0(job: &mut PassJob, tick: u32) -> (u64, bool) {
        boom_when(job, tick, "a", 0)
    }

    fn boom_c_at_last(job: &mut PassJob, tick: u32) -> (u64, bool) {
        boom_when(job, tick, "c", 7)
    }

    fn three_jobs() -> Vec<PassJob> {
        ["a", "b", "c"]
            .map(|n| poll_job(n, Engine::Interp))
            .into_iter()
            .collect()
    }

    #[test]
    fn threads_run_every_tick_on_every_core_and_return_the_cores() {
        let jobs = vec![poll_job("a", Engine::Interp), poll_job("b", Engine::Interp)];
        let r = run_ticks(jobs, 2, 5).unwrap();
        assert_eq!(r.tick_ns.len(), 7);
        for c in &r.cores {
            assert_eq!(c.steps.len(), 7, "{}", c.name);
            assert_eq!(c.ns.len(), 7);
        }
        assert_eq!(r.jobs.len(), 2);
    }

    #[test]
    fn an_fx_job_gets_its_drive_before_every_tick() {
        let mut j = poll_job("fx2", Engine::Interp);
        j.fx = Some(fx_run::FxDrive::new(fx_run::FX2));
        let r = run_ticks(vec![j], 0, 4).unwrap();
        // counter shown for pass 3 is 3, the core's own copy is counter + 1
        let mut c = r.jobs.into_iter().next().unwrap().core;
        assert_eq!(c.dm_read(fx_run::SELF_COUNTER), Ok(4));
    }

    #[test]
    fn a_panic_in_one_cores_pass_ends_the_run_with_a_message_and_never_hangs() {
        // 2 warm-up passes and 6 measured: ticks 0..8; `b` panics in tick 3
        let e = match bounded(three_jobs(), 2, 6, boom_b_at_3) {
            Ok(_) => panic!("a panicking core is an error, not a report"),
            Err(e) => e,
        };
        assert!(
            e.contains("core b") && e.contains("pass 3") && e.contains("boom in b"),
            "{e}"
        );
    }

    #[test]
    fn a_panic_in_the_first_or_the_last_pass_ends_the_run_too() {
        let e = bounded(three_jobs(), 2, 6, boom_a_at_0).err().unwrap();
        assert!(e.contains("core a") && e.contains("pass 0"), "{e}");
        let e = bounded(three_jobs(), 2, 6, boom_c_at_last).err().unwrap();
        assert!(e.contains("core c") && e.contains("pass 7"), "{e}");
    }

    #[test]
    fn a_run_without_a_panic_is_unchanged_by_the_guard() {
        let r = bounded(three_jobs(), 1, 3, |job, _| job.run_pass()).unwrap();
        assert_eq!((r.cores.len(), r.jobs.len(), r.tick_ns.len()), (3, 3, 4));
        assert!(r.cores.iter().all(|c| c.steps.len() == 4));
    }

    #[test]
    fn payloads_of_every_kind_have_a_text() {
        assert_eq!(panic_text(&"a literal"), "a literal");
        assert_eq!(panic_text(&String::from("a formatted 3")), "a formatted 3");
        assert!(panic_text(&42u32).contains("without a text"));
    }

    /// The synthetic poll job armed for `engine`.
    fn armed_job(engine: Engine) -> PassJob {
        let preset = synthetic_preset();
        let mut core = boot_core(&synthetic_image(), &preset, &PeriphOpts::default()).unwrap();
        arm_for(&mut core, engine, 0, preset.idle_pc, None).unwrap();
        PassJob::from_preset("synthetic", core, &preset, Vec::new(), engine)
    }

    #[test]
    fn the_counters_are_snapshotted_between_the_warm_up_and_the_measured_passes() {
        let r = run_ticks(vec![armed_job(Engine::Uop)], 3, 5).unwrap();
        let (series, end) = (&r.cores[0], r.jobs[0].core.uop_stats());
        assert_eq!(series.uop_at_warmup.passes, 3, "the warm-up passes");
        assert_eq!(end.passes, 8);
        let measured = end.since(&series.uop_at_warmup);
        assert_eq!(measured.passes, 5);
        assert_eq!(
            measured.steps_uop + measured.steps_core_step,
            5 * SYNTH_PASS_STEPS,
            "every measured step is accounted once and no warm-up step is in it"
        );
        assert_eq!(
            measured.translations, 0,
            "nothing was retranslated in a steady run"
        );
    }

    #[test]
    fn without_a_warm_up_the_snapshot_is_the_armed_state() {
        let armed = armed_job(Engine::Uop);
        let built = armed.core.uop_stats();
        assert!(
            built.translations > 0,
            "arming translated the code: {built:?}"
        );
        let r = run_ticks(vec![armed], 0, 2).unwrap();
        assert_eq!(r.cores[0].uop_at_warmup, built);
        let measured = r.jobs[0].core.uop_stats().since(&r.cores[0].uop_at_warmup);
        assert_eq!((measured.passes, measured.translations), (2, 0));
    }

    #[test]
    fn a_run_with_no_measured_pass_has_all_of_it_in_the_snapshot() {
        let r = run_ticks(vec![armed_job(Engine::Uop)], 2, 0).unwrap();
        assert_eq!(r.cores[0].uop_at_warmup, r.jobs[0].core.uop_stats());
        assert_eq!(r.cores[0].uop_at_warmup.passes, 2);
    }

    #[test]
    fn an_engine_without_counters_has_an_empty_snapshot() {
        let r = run_ticks(vec![poll_job("i", Engine::Interp)], 2, 2).unwrap();
        assert_eq!(r.cores[0].uop_at_warmup, crate::core::UopStats::default());
    }
}
