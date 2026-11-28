//! OWNER: W1-VGEN. Schedules: the outer loop a tier has to reproduce. A program runs in slices of
//! different budgets with `until` and `stop_pc` addresses, and between two slices a hook is
//! added (which flags the page and queues its decoded code for invalidation).
//!
//! [`schedule_vs_oracle`] runs a schedule on the oracle (T0) and on any other driver from equal
//! machines and compares the whole observable state after every slice, so the first slice that
//! differs names itself. It returns what the oracle did ([`SchedStats`]) so a test can prove it
//! exercised something (the rule of `testkit/diffrun.rs`).

use super::program::Program;
use crate::machine::sb::testkit::{compare_machines, Rng};
use crate::machine::{Hook, Machine, Stop};

/// One call of `Machine::run`, and the hook added before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slice {
    pub budget: u64,
    /// Stop address (0: none, no program runs at 0).
    pub until: u32,
    pub stop: Option<u32>,
    pub hook: Option<(u32, Hook)>,
}

/// The slices of one run, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Schedule {
    pub slices: Vec<Slice>,
}

/// How a tier runs one slice: `(machine, budget, until, stop_pc) -> (instructions, why)`, the
/// shape of `Machine::run` and of `testgen::drive_traces`.
pub(crate) type Driver<'a> = dyn FnMut(&mut Machine, u64, u32, Option<u32>) -> (u64, Stop) + 'a;

/// What the oracle did over a schedule.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SchedStats {
    pub slices: u64,
    pub retired: u64,
    /// Hooks added.
    pub hooks: u64,
    /// Slices that ended on the budget, `until`, `stop_pc`, a fault or a hook loop.
    pub budget: u64,
    pub until: u64,
    pub stop_pc: u64,
    pub faults: u64,
    pub hook_loops: u64,
}

impl SchedStats {
    fn note(&mut self, n: u64, why: &Stop) {
        self.slices += 1;
        self.retired += n;
        match why {
            Stop::Budget => self.budget += 1,
            Stop::Until => self.until += 1,
            Stop::StopPc => self.stop_pc += 1,
            Stop::Fault(_) => self.faults += 1,
            Stop::HookLoop(_) => self.hook_loops += 1,
        }
    }

    pub fn merge(&mut self, o: &Self) {
        self.slices += o.slices;
        self.retired += o.retired;
        self.hooks += o.hooks;
        self.budget += o.budget;
        self.until += o.until;
        self.stop_pc += o.stop_pc;
        self.faults += o.faults;
        self.hook_loops += o.hook_loops;
    }

    /// Panic with `what` unless at least `slices` slices retired `retired` instructions in all.
    #[track_caller]
    pub fn assert_exercised(&self, what: &str, slices: u64, retired: u64) {
        assert!(
            self.slices >= slices && self.retired >= retired,
            "{what}: vacuous schedule run {self:?} (need {slices} slices, {retired} instructions)"
        );
    }
}

/// The machines after a successful [`schedule_vs_oracle`] and what the oracle did.
pub(crate) struct SchedRun {
    pub stats: SchedStats,
    pub want: Machine,
    pub got: Machine,
}

/// `n` slices for `p`: budgets from one instruction to a few thousand (three in ten below 20),
/// an `until` and a `stop_pc` at a mark of the program now and then, and in one slice of three a
/// hook at a mark. The dense sweep of every small budget is [`super::budget_sweep`].
pub(crate) fn random_schedule(rng: &mut Rng, p: &Program, n: usize) -> Schedule {
    let slices = (0..n)
        .map(|_| {
            let budget = match rng.below(10) {
                0..=2 => rng.range(1, 20),
                3..=6 => rng.range(20, 200),
                7..=8 => rng.range(200, 1000),
                _ => rng.range(1000, 3000),
            } as u64;
            let until = if rng.chance(5) { rng.pick(&p.marks) } else { 0 };
            let stop = rng.chance(5).then(|| rng.pick(&p.marks));
            let hook = rng.chance(3).then(|| {
                let at = rng.pick(&p.marks);
                let hook = match rng.below(7) {
                    0 | 1 => Hook::Return(rng.biased()),
                    2 => Hook::SkipInsn,
                    3 => Hook::ZeroR5,
                    4 => Hook::BoardPresent,
                    5 => Hook::BumpNew,
                    _ => Hook::BoardConfig,
                };
                (at, hook)
            });
            Slice {
                budget,
                until,
                stop,
                hook,
            }
        })
        .collect();
    Schedule { slices }
}

/// Run `s` on `m` with `drive`: add each slice's hook, run the slice, record its result. Ends
/// early after a fault or a hook loop (the machine is not meant to go on after those).
pub(crate) fn run_schedule(m: &mut Machine, s: &Schedule, drive: &mut Driver) -> Vec<(u64, Stop)> {
    let mut out = Vec::new();
    for sl in &s.slices {
        if let Some((at, h)) = sl.hook {
            m.add_hook(at, h);
        }
        let res = drive(m, sl.budget, sl.until, sl.stop);
        out.push(res);
        if matches!(res.1, Stop::Fault(_) | Stop::HookLoop(_)) {
            break;
        }
    }
    out
}

/// Run `s` on a clone of `m0` with `predecode = false` (the oracle) and on a clone with `drive`;
/// after every slice compare instruction count, stop reason and the whole snapshot.
/// `Err` names the slice and the first difference.
pub(crate) fn schedule_vs_oracle(
    m0: &Machine,
    s: &Schedule,
    drive: &mut Driver,
) -> Result<SchedRun, String> {
    let (mut want, mut got) = (m0.clone(), m0.clone());
    want.predecode = false;
    let mut stats = SchedStats::default();
    for (i, sl) in s.slices.iter().enumerate() {
        if let Some((at, h)) = sl.hook {
            want.add_hook(at, h);
            got.add_hook(at, h);
            stats.hooks += 1;
        }
        let w = want.run(sl.budget, sl.until, sl.stop);
        let g = drive(&mut got, sl.budget, sl.until, sl.stop);
        stats.note(w.0, &w.1);
        compare_machines(&want, w, &got, g).map_err(|e| format!("slice {i} {sl:?}: {e}"))?;
        if matches!(w.1, Stop::Fault(_) | Stop::HookLoop(_)) {
            break;
        }
    }
    Ok(SchedRun { stats, want, got })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testgen::program::{machine_from, structured_program};
    use crate::machine::sb::testgen::{drive_traces, CODE};

    fn setup(seed: u64) -> (Program, Machine, Schedule) {
        let mut rng = Rng::new(seed);
        let p = structured_program(&mut rng, 60 + (seed as usize % 7) * 10);
        let m = machine_from(&p, true);
        let s = random_schedule(&mut rng, &p, 8);
        (p, m, s)
    }

    fn t1(m: &mut Machine, n: u64, until: u32, stop: Option<u32>) -> (u64, Stop) {
        m.run(n, until, stop)
    }

    #[test]
    fn schedules_are_deterministic_per_seed_and_stay_inside_the_program() {
        let (p, _, a) = setup(5);
        assert_eq!(a, setup(5).2);
        assert_ne!(a, setup(6).2);
        for sl in &a.slices {
            assert!(sl.budget >= 1 && (sl.until == 0 || p.marks.contains(&sl.until)));
            assert!(sl.stop.is_none_or(|s| p.marks.contains(&s)));
            assert!(sl.hook.is_none_or(|(at, _)| p.marks.contains(&at)));
        }
        let all: Vec<Slice> = (0..40).flat_map(|s| setup(s).2.slices).collect();
        assert!(all.iter().any(|s| s.budget < 9) && all.iter().any(|s| s.budget >= 1000));
        assert!(
            all.iter().any(|s| (9..20).contains(&s.budget)),
            "the budgets between 9 and 19 are drawn too"
        );
        assert!(all.iter().any(|s| s.hook.is_some()) && all.iter().any(|s| s.stop.is_some()));
        assert!(all.iter().any(|s| s.until != 0));
    }

    #[test]
    fn the_oracle_against_itself_agrees_and_counts_what_it_did() {
        let mut total = SchedStats::default();
        for seed in 0..30 {
            let (_, m, s) = setup(seed);
            let run = schedule_vs_oracle(&m, &s, &mut |m, n, u, st| {
                m.predecode = false;
                m.run(n, u, st)
            })
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            total.merge(&run.stats);
            assert!(run.want.cpu.insns > 0 && run.want.cpu.insns == run.got.cpu.insns);
        }
        total.assert_exercised("oracle against itself", 100, 3000);
        assert!(
            total.hooks >= 20 && total.until >= 1 && total.stop_pc >= 1,
            "{total:?}"
        );
    }

    #[test]
    fn a_driver_that_does_something_else_is_named_by_slice_and_field() {
        let (_, m, s) = setup(2);
        let e = schedule_vs_oracle(&m, &s, &mut |m, n, u, st| t1(m, n + 1, u, st))
            .err()
            .expect("a driver that runs one more instruction differs");
        assert!(e.starts_with("slice ") && e.contains("insns"), "{e}");
        let e = schedule_vs_oracle(&m, &s, &mut |m, n, u, st| {
            let res = m.run(n, u, st);
            m.cpu.r[4] ^= 1;
            res
        })
        .err()
        .expect("a driver that corrupts r4 differs");
        assert!(e.contains("r4"), "{e}");
        let e = schedule_vs_oracle(&m, &s, &mut |m, n, u, st| {
            let res = m.run(n, u, st);
            m.bus.mem.write32(0x9000_0010, 0x77);
            res
        })
        .err()
        .expect("a driver that writes memory differs");
        assert!(e.contains("mem_hash") && e.contains("[0x90000010]"), "{e}");
    }

    #[test]
    fn run_schedule_adds_the_hooks_and_ends_at_a_fault() {
        let (_, m, s) = setup(4);
        let mut a = m.clone();
        let res = run_schedule(&mut a, &s, &mut |m, n, u, st| m.run(n, u, st));
        assert!(!res.is_empty() && res.len() <= s.slices.len());
        let hooks = s
            .slices
            .iter()
            .take(res.len())
            .filter(|s| s.hook.is_some())
            .count();
        assert!(a.hooks.len() <= hooks && (hooks == 0 || !a.hooks.is_empty()));
        let faulty = Schedule {
            slices: vec![
                Slice {
                    budget: 5,
                    until: 0,
                    stop: None,
                    hook: None,
                },
                Slice {
                    budget: 5,
                    until: 0,
                    stop: None,
                    hook: None,
                },
            ],
        };
        let mut f = Machine::new();
        f.bus.mem.write32(CODE, 0xE7F0_00F0);
        f.cpu.r[15] = CODE;
        let res = run_schedule(&mut f, &faulty, &mut |m, n, u, st| m.run(n, u, st));
        assert_eq!(res.len(), 1, "the schedule ends at the fault");
        assert!(matches!(res[0].1, Stop::Fault(_)));
    }

    #[test]
    fn tier_1_and_the_skeleton_trace_driver_equal_the_oracle_on_generated_programs() {
        let (mut t1_stats, mut t2_stats, mut invalidated) =
            (SchedStats::default(), SchedStats::default(), 0);
        for seed in 0..40 {
            let (p, m, s) = setup(seed);
            let run = schedule_vs_oracle(&m, &s, &mut |m, n, u, st| m.run(n, u, st))
                .unwrap_or_else(|e| panic!("T1, seed {seed}: {e}"));
            t1_stats.merge(&run.stats);
            invalidated += u32::from(run.got.code.invalidated > 0);
            let m2 = machine_from(&p, false);
            let run = schedule_vs_oracle(&m2, &s, &mut |m, n, u, st| drive_traces(m, n, u, st))
                .unwrap_or_else(|e| panic!("trace driver, seed {seed}: {e}"));
            t2_stats.merge(&run.stats);
        }
        t1_stats.assert_exercised("T1 against the oracle", 150, 10_000);
        t2_stats.assert_exercised("trace driver against the oracle", 150, 10_000);
        assert!(
            invalidated >= 3,
            "T1 had decoded code invalidated in {invalidated} of 40 runs"
        );
        assert!(
            t1_stats.hooks >= 30 && t1_stats.stop_pc + t1_stats.until >= 3,
            "{t1_stats:?}"
        );
    }
}
