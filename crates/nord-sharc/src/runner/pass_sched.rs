//! Lock-free 1-thread-per-core pass scheduler (bean `ns4-lle3`): every SHARC core of the system
//! owns an OS thread, all threads start tick `t` together on a spin barrier and each runs one
//! audio pass on its own [`Core`]. There is no mutex and no channel in the hot path: the
//! coordinator publishes the tick number in one atomic, each worker spins on it, runs its pass and
//! increments a second atomic, and the coordinator spins until all workers have counted in.
//!
//! The cores of the real system exchange audio through the SPORT/DMA bus (`ns4-lle4`); this
//! scheduler measures the compute side only: how long each core takes for a pass of its own
//! (latency per core, and the tick wall time = the slowest core plus the barrier) when all cores
//! run at the same time on separate host cores.
use super::fx_run::FxDrive;
use super::msg::{self, Msg};
use super::pass::PassConfig;
use super::system::{lean_pass, refresh_inputs, set_ping_pong};
use crate::core::Core;
use crate::cpu_clock::ThreadClock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// How a core runs its pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// The interpreter ([`Core::step`] to the poll loop).
    Interp,
    /// The whole-pass AOT blocks enabled on the core (`Core::enable_aot_tag`), the interpreter for
    /// any pc outside them.
    Aot,
}

/// [`lean_pass`] with the AOT block hook: blocks run whole, the interpreter fills the gaps.
pub fn lean_pass_aot(core: &mut Core, cfg: &PassConfig) -> (u64, bool) {
    super::system::start(core, cfg);
    let start = core.steps;
    let limit = start + cfg.max_steps;
    use crate::core::aot_prof::{
        aot_prof_on, aot_prof_pass, aot_prof_step, tsc, MID_BLOCK, REFUSED, UNBLOCKED,
    };
    let prof = aot_prof_on();
    let (t_pass, mut try_cycles) = (if prof { tsc() } else { 0 }, 0u64);
    while core.steps < limit {
        let t = if prof { tsc() } else { 0 };
        let entered = core.aot_try(limit);
        if entered {
            if prof {
                try_cycles += tsc() - t;
            }
            if core.aot_take_fault().is_some() {
                return (core.steps - start, false);
            }
        } else if prof {
            let cause = if core.aot_at_block_start() {
                REFUSED
            } else if core.aot_pc_in_block() {
                MID_BLOCK
            } else {
                UNBLOCKED
            };
            let (pc, t) = (core.pc, tsc());
            let r = core.step();
            aot_prof_step(pc, cause, tsc() - t);
            if r.is_err() {
                return (core.steps - start, false);
            }
        } else if core.step().is_err() {
            return (core.steps - start, false);
        }
        if core.pc == cfg.idle_pc && core.dm_read(cfg.trigger) == Ok(0) {
            if prof {
                aot_prof_pass(tsc() - t_pass, try_cycles);
            }
            return (core.steps - start, true);
        }
    }
    (core.steps - start, false)
}

/// One core of the system and what it needs to run passes.
pub struct PassJob {
    pub name: String,
    pub core: Core,
    pub cfg: PassConfig,
    /// ARM messages delivered before the pass they name (`msg_base` as in `sharc-sys`).
    pub msgs: Vec<Msg>,
    pub msg_base: u32,
    pub ping_pong: Option<(u32, u32, u32)>,
    /// FX chip core: the Core1 hand-over counter and ping selector before every pass.
    pub fx: Option<FxDrive>,
    pub engine: Engine,
}

impl PassJob {
    /// A job from a booted core and its start-up preset (poll loop, mailbox, ping-pong, FX drive).
    pub fn from_preset(
        name: &str,
        core: Core,
        preset: &super::live_presets::Preset,
        msgs: Vec<Msg>,
        engine: Engine,
    ) -> Self {
        Self {
            name: name.into(),
            core,
            cfg: PassConfig {
                trigger: preset.trigger,
                idle_pc: preset.idle_pc,
                max_steps: 2_000_000,
                irq: preset.irq,
            },
            msgs,
            msg_base: 0,
            ping_pong: preset.ping_pong,
            fx: preset.fx.map(FxDrive::new),
            engine,
        }
    }

    /// Everything that happens before pass `p`: the ping-pong pointer, the ARM messages due
    /// (`next_msg` indexes `msgs`, which must be sorted by pass), the FX hand-over words and the
    /// input refresh. The first message that cannot be stored is returned after all were tried.
    pub fn before_pass(&mut self, p: u32, next_msg: &mut usize) -> Result<(), String> {
        let mut err = Ok(());
        if let Some(pp) = self.ping_pong {
            set_ping_pong(&mut self.core, pp, p);
        }
        while *next_msg < self.msgs.len() && self.msgs[*next_msg].pass <= p {
            if let Err(e) = msg::deliver(&mut self.core, self.msg_base, &self.msgs[*next_msg]) {
                err = Err(e);
            }
            *next_msg += 1;
        }
        if let Some(fx) = self.fx.as_mut() {
            fx.before_pass(&mut self.core);
        }
        refresh_inputs(&mut self.core);
        err
    }

    /// Runs the pass on the job's engine; `(steps, returned to the poll loop)`.
    pub fn run_pass(&mut self) -> (u64, bool) {
        match self.engine {
            Engine::Interp => lean_pass(&mut self.core, &self.cfg),
            Engine::Aot => lean_pass_aot(&mut self.core, &self.cfg),
        }
    }
}

/// What one core measured.
#[derive(Debug, Clone, Default)]
pub struct CoreSeries {
    pub name: String,
    /// Steps of each pass (warm-up included).
    pub steps: Vec<u64>,
    /// Pass latency in ns (the pass only, not the message delivery), warm-up included.
    pub ns: Vec<u64>,
    /// Every pass returned to its poll loop.
    pub all_idle: bool,
}

/// Result of a run: per-core series, tick wall times (publish to last core done) and the cores.
pub struct SchedReport {
    pub cores: Vec<CoreSeries>,
    pub tick_ns: Vec<u64>,
    pub jobs: Vec<PassJob>,
}

/// Runs `warmup + ticks` ticks of all `jobs` concurrently, one thread per job.
pub fn run_ticks(jobs: Vec<PassJob>, warmup: u32, ticks: u32) -> Result<SchedReport, String> {
    let total = u64::from(warmup + ticks);
    let n = jobs.len() as u32;
    // calibrate the thread CPU clock now (200 ms on the first call), not inside the first tick
    // where it would stall every core at the barrier
    let _ = crate::cpu_clock::ns_per_tick();
    let epoch = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicU32::new(0));
    let mut handles = Vec::new();
    for mut job in jobs {
        let (epoch, done) = (epoch.clone(), done.clone());
        handles.push(std::thread::spawn(move || {
            let mut series = CoreSeries {
                name: job.name.clone(),
                all_idle: true,
                ..CoreSeries::default()
            };
            let mut next_msg = 0;
            job.msgs.sort_by_key(|m| m.pass);
            for tick in 0..total {
                let mut spins = 0u32;
                while epoch.load(Ordering::Acquire) <= tick {
                    spins += 1;
                    if spins < 1 << 16 {
                        std::hint::spin_loop();
                    } else {
                        std::thread::yield_now();
                    }
                }
                let p = tick as u32;
                if job.before_pass(p, &mut next_msg).is_err() {
                    series.all_idle = false;
                }
                // thread CPU time of the pass: a descheduled thread (other builds on the host) does
                // not inflate the per-core figure; the barrier tick below stays wall time
                let t0 = ThreadClock::start();
                let (steps, idle) = job.run_pass();
                let ns = t0.elapsed_ns();
                series.steps.push(steps);
                series.ns.push(ns);
                series.all_idle &= idle;
                done.fetch_add(1, Ordering::AcqRel);
            }
            (series, job)
        }));
    }
    let mut tick_ns = Vec::with_capacity(total as usize);
    for tick in 0..total {
        done.store(0, Ordering::Release);
        let t0 = Instant::now();
        epoch.store(tick + 1, Ordering::Release);
        let mut spins = 0u32;
        while done.load(Ordering::Acquire) < n {
            spins += 1;
            if spins < 1 << 16 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
        tick_ns.push(t0.elapsed().as_nanos() as u64);
    }
    let mut cores = Vec::new();
    let mut jobs = Vec::new();
    for h in handles {
        let (s, j) = h.join().map_err(|_| "a core thread panicked".to_string())?;
        cores.push(s);
        jobs.push(j);
    }
    Ok(SchedReport {
        cores,
        tick_ns,
        jobs,
    })
}

/// Mean, median, p99 and worst of `v` (ns), after dropping the first `skip` samples.
pub fn summarize(v: &[u64], skip: usize) -> (f64, f64, f64, f64) {
    let mut s: Vec<u64> = v.iter().skip(skip).copied().collect();
    if s.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    s.sort_unstable();
    let mean = s.iter().sum::<u64>() as f64 / s.len() as f64;
    let at = |q: f64| s[((s.len() - 1) as f64 * q).round() as usize] as f64;
    (mean, at(0.5), at(0.99), s[s.len() - 1] as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    /// A synthetic "pass": the poll loop at `idle` waits for the trigger word, a handler at `h`
    /// does a few adds and jumps back. Built from raw parcels, no firmware.
    fn poll_job(name: &str, engine: Engine) -> PassJob {
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

    #[test]
    fn summarize_orders_percentiles() {
        let v: Vec<u64> = (1..=100).collect();
        let (mean, med, p99, worst) = summarize(&v, 0);
        assert_eq!((mean, worst), (50.5, 100.0));
        assert!((med - 50.0).abs() <= 1.0 && p99 >= 99.0);
        assert_eq!(summarize(&v, 100), (0.0, 0.0, 0.0, 0.0));
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
        j.fx = Some(FxDrive::new(super::super::fx_run::FX2));
        let r = run_ticks(vec![j], 0, 4).unwrap();
        // counter shown for pass 3 is 3, the core's own copy is counter + 1
        let mut c = r.jobs.into_iter().next().unwrap().core;
        assert_eq!(c.dm_read(super::super::fx_run::SELF_COUNTER), Ok(4));
    }

    #[test]
    fn aot_engine_without_blocks_equals_the_interpreter() {
        let mut a = poll_job("i", Engine::Interp);
        let mut b = poll_job("a", Engine::Aot);
        let (sa, ia) = lean_pass(&mut a.core, &a.cfg);
        let (sb, ib) = lean_pass_aot(&mut b.core, &b.cfg);
        assert_eq!((sa, ia), (sb, ib));
        assert_eq!(a.core.state_mismatch(&b.core), None);
    }
}
