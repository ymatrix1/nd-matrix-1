//! Test fixtures of the scheduler modules: a job that needs no firmware.
//!
//! OWNER: W1-RUN. Hand-written parcels.
use super::{Engine, PassJob};
use crate::core::support::core_with;
use crate::runner::pass::PassConfig;

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
}
