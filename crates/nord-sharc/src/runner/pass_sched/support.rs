//! Test fixtures of the scheduler modules: a job that needs no firmware, one whose engine is
//! wrong, and the run with a deadline.
//!
//! OWNER: W1-RUN. Hand-written parcels.
use super::{run_ticks_with, Engine, PassFn, PassJob, SchedReport};
use crate::core::support::core_with;
use crate::runner::pass::PassConfig;
use crate::runner::uop_pass::support::mark_wrong_engine;
use std::time::Duration;

/// A synthetic "pass": four NOPs at the code start, which is also the idle pc and so ends the pass
/// when a step lands back on it. Built from raw parcels, no firmware.
pub fn poll_job(name: &str, engine: Engine) -> PassJob {
    // 0x120000: NOP; NOP (the run starts at the entry and stays in this tiny loop)
    let core = core_with(&[0x0001, 0x0001, 0x0001, 0x0001]);
    PassJob {
        name: name.into(),
        core,
        cfg: PassConfig {
            trigger: 0xB_00E2,
            idle_pc: 0x12_0000,
            max_steps: 1000,
            irq: None,
        },
        msgs: Vec::new(),
        msg_base: 0,
        ping_pong: None,
        fx: None,
        engine,
    }
}

/// [`poll_job`] whose uop engine is wrong (`uop_pass::support::mark_wrong_engine`): every pass
/// it runs on a uop engine ends with a register flipped, which only [`Engine::UopChecked`]
/// notices. The engine is exact against the interpreter by construction and by test, so this is
/// how a test gets an engine that differs from it.
pub fn wrong_poll_job(name: &str, engine: Engine) -> PassJob {
    let mut job = poll_job(name, engine);
    mark_wrong_engine(&mut job.core);
    job
}

/// [`run_ticks_with`] on a helper thread with a deadline: a regression that hangs the barrier
/// fails the test with a message instead of hanging the test run.
pub fn bounded(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_job_runs_on_the_engine_it_is_given() {
        for engine in [Engine::Interp, Engine::Uop, Engine::Aot] {
            let j = poll_job("j", engine);
            assert_eq!((j.name.as_str(), j.engine), ("j", engine));
            assert_eq!(j.cfg.idle_pc, 0x12_0000);
        }
    }

    #[test]
    fn a_wrong_job_runs_the_same_pass_and_ends_in_another_state() {
        let (mut plain, mut wrong) = (poll_job("p", Engine::Uop), wrong_poll_job("w", Engine::Uop));
        assert_eq!((wrong.name.as_str(), wrong.engine), ("w", Engine::Uop));
        assert_eq!(plain.run_pass(), wrong.run_pass(), "the same pass");
        assert!(plain.core.state_mismatch(&wrong.core).is_some(), "the mark");
    }

    #[test]
    fn a_bounded_run_returns_the_report_of_the_run() {
        let r = bounded(vec![poll_job("j", Engine::Interp)], 1, 2, |job, _| {
            job.run_pass()
        })
        .unwrap();
        assert_eq!((r.cores.len(), r.tick_ns.len()), (1, 3));
    }
}
