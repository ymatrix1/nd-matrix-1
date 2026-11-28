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
//!
//! This file holds the engines, the job and the report types; the threaded loop is in [`sched`]
//! (re-exported here), where a panic in one core's pass ends the whole run with a message instead
//! of leaving the barrier waiting for a count that never comes.
mod sched;
#[cfg(test)]
mod support;

use super::fx_run::FxDrive;
use super::msg::{self, Msg};
use super::pass::PassConfig;
use super::system::{lean_pass, refresh_inputs, set_ping_pong};
use super::uop_pass::{lean_pass_uop, lean_pass_uop_checked};
use crate::core::{Core, UopStats};

pub use sched::{run_ticks, run_ticks_with, PassFn};

/// How a core runs its pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Engine {
    /// The interpreter ([`Core::step`] to the poll loop). The default.
    #[default]
    Interp,
    /// The whole-pass AOT blocks enabled on the core (`Core::enable_aot_tag`), the interpreter for
    /// any pc outside them.
    Aot,
    /// The uop engine, Tier 1 ([`Core::enable_uop`], D260, `docs/specs/sharc-uop.md`): translated
    /// blocks run through the handler table, the interpreter fills the gaps.
    Uop,
    /// [`Engine::Uop`] checked against the interpreter ([`lean_pass_uop_checked`]); a difference
    /// panics, which [`run_ticks`] turns into an error that names the core and the pass.
    /// Allocates, for tests and tools only.
    UopChecked,
    /// [`Engine::Uop`] on a core enabled with `UopConfig::native` (the Cranelift tier, feature
    /// `uop-native`); the pass loop is the same, only the enabling differs.
    UopNative,
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
            Engine::Uop | Engine::UopNative => lean_pass_uop(&mut self.core, &self.cfg),
            Engine::UopChecked => {
                let (steps, idle, diff) = lean_pass_uop_checked(&mut self.core, &self.cfg);
                if let Some(d) = diff {
                    panic!("uop pass differs from the interpreter: {d}");
                }
                (steps, idle)
            }
        }
    }
}

/// What one core measured.
#[derive(Debug, Clone, Default)]
pub struct CoreSeries {
    pub name: String,
    /// Steps of each pass (warm-up included).
    pub steps: Vec<u64>,
    /// Pass latency in WALL ns (the pass only, not the message delivery), warm-up included. The
    /// "x of budget" claim uses this.
    pub ns: Vec<u64>,
    /// The same passes in thread CPU ns (equal to `ns` where there is no thread counter, see
    /// `cpu_clock::ThreadClock::is_cpu_time`); a loaded host does not inflate it.
    pub cpu_ns: Vec<u64>,
    /// Every pass returned to its poll loop.
    pub all_idle: bool,
    /// The uop engine counters when the first measured pass was about to start (after the warm-up
    /// passes, before their messages), all zero for an engine without counters. The counters the
    /// run ended with minus this snapshot ([`UopStats::since`]) are the measured passes only: the
    /// steady-state gates (`maintain_calls == 0`, `smc == 0`) must not see the warm-up.
    pub uop_at_warmup: UopStats,
}

/// Result of a run: per-core series, tick wall times (publish to last core done) and the cores.
pub struct SchedReport {
    pub cores: Vec<CoreSeries>,
    pub tick_ns: Vec<u64>,
    pub jobs: Vec<PassJob>,
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
    use super::support::{bounded, poll_job, wrong_poll_job};
    use super::*;

    #[test]
    #[should_panic(expected = "uop pass differs from the interpreter: r:")]
    fn the_checked_engine_panics_when_the_engine_differs_from_the_interpreter() {
        // the arm of `run_pass` itself: an engine that ends a pass in another state is not
        // reported and carried on, it stops the pass with the difference in the message
        let mut job = wrong_poll_job("w", Engine::UopChecked);
        job.run_pass();
    }

    #[test]
    fn the_other_uop_engines_do_not_check_so_a_wrong_engine_goes_unseen_there() {
        let mut want = poll_job("i", Engine::Interp);
        let steps = want.run_pass();
        for engine in [Engine::Uop, Engine::UopNative] {
            let mut job = wrong_poll_job("w", engine);
            assert_eq!(job.run_pass(), steps, "{engine:?} runs the pass, unchecked");
            assert!(want.core.state_mismatch(&job.core).is_some(), "{engine:?}");
        }
    }

    #[test]
    fn a_wrong_engine_under_run_ticks_ends_the_run_with_the_core_and_the_difference() {
        // the whole path of `lle_sharc_bench --engine uop-checked`: the scheduler's worker
        // thread, `run_pass`, the checked pass and the panic that `run_ticks` turns into an
        // error (the benchmark's `main` returns it, so the exit status is not zero). The other
        // core is a good one, so the run must also stop it instead of waiting for it.
        let jobs = vec![
            poll_job("good", Engine::UopChecked),
            wrong_poll_job("bad", Engine::UopChecked),
        ];
        let e = bounded(jobs, 1, 3, |job, _| job.run_pass()).err().unwrap();
        assert!(
            e.contains("core bad panicked in pass 0")
                && e.contains("uop pass differs from the interpreter: r:"),
            "{e}"
        );
        assert!(!e.contains("core good"), "{e}");
    }

    #[test]
    fn a_good_checked_engine_runs_to_the_end_under_run_ticks() {
        let jobs = vec![poll_job("a", Engine::UopChecked)];
        let r = bounded(jobs, 1, 3, |job, _| job.run_pass()).unwrap();
        assert_eq!((r.cores.len(), r.cores[0].steps.len()), (1, 4));
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
    fn engine_default_is_interp() {
        assert_eq!(Engine::default(), Engine::Interp);
        assert_eq!(poll_job("d", Engine::default()).engine, Engine::Interp);
    }

    #[test]
    fn uop_engines_without_blocks_equal_the_interpreter() {
        let mut a = poll_job("i", Engine::Interp);
        let want = a.run_pass();
        for engine in [Engine::Uop, Engine::UopChecked, Engine::UopNative] {
            let mut b = poll_job("u", engine);
            assert_eq!(b.run_pass(), want, "{engine:?}");
            assert_eq!(a.core.state_mismatch(&b.core), None, "{engine:?}");
        }
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
