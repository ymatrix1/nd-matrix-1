//! OWNER: W2-PEEP (tests). The L0/L1 differential harness of the three peephole passes (design
//! note 8): each pass preserves state against the unpassed trace and the interpreter.
//!
//! * [`check_pair`] (L0 style, random states): build the trace at one pc twice, passes off and
//!   on; require equal exits (kind, register, `done`, target, `last_pc`), `n_guest`, words and
//!   `Uop::n` sums, well-formed predicates; run both with `exec::run_sb` from one random state
//!   on a logging bus, and the interpreter for as many instructions, and require equal registers,
//!   flags, fault, `bus.pc`, exit, budget left and ordered access log (with pcs).
//! * [`program_check`] (L1 style): a whole program for a budget on the interpreter and on traces
//!   built with the passes (driver contract of `testgen/drive.rs`), compared on the full machine
//!   snapshot: loop-backs, budgets expiring inside a trace, stale pages and `until`.
//!
//! Every comparison fills a [`Tally`] the tests assert on (a pass that never fired proves nothing).

use crate::cpu::{Cpu, Fault};
use crate::machine::sb::exec::run_sb;
use crate::machine::sb::testgen::{machine_with, CODE};
use crate::machine::sb::testkit::oracle::oracle_trace_on;
use crate::machine::sb::testkit::{
    compare_machines, oracle_run, random_cpu_flags, CpuSnap, DiffRun, LogBus, Rng, TraceRun,
};
use crate::machine::sb::trace::{build, BuildOpts};
use crate::machine::sb::uop::{k, Sb, Uop};
use crate::machine::{Machine, Stop};
use crate::mem::{Mem, HOOK_X};

/// Which passes a trace is built with.
#[derive(Clone, Copy, Debug)]
pub(in crate::machine::sb) struct Passes {
    pub fuse: bool,
    pub dead_flags: bool,
    pub predicate: bool,
}

impl Passes {
    pub const NONE: Passes = Passes {
        fuse: false,
        dead_flags: false,
        predicate: false,
    };
    pub const FUSE: Passes = Passes {
        fuse: true,
        ..Self::NONE
    };
    pub const DEAD: Passes = Passes {
        dead_flags: true,
        ..Self::NONE
    };
    pub const PRED: Passes = Passes {
        predicate: true,
        ..Self::NONE
    };
    pub const ALL: Passes = Passes {
        fuse: true,
        dead_flags: true,
        predicate: true,
    };

    pub fn opts(self) -> BuildOpts {
        BuildOpts {
            fuse: self.fuse,
            dead_flags: self.dead_flags,
            predicate: self.predicate,
            fold_literals: true,
            ..BuildOpts::default()
        }
    }
}

/// What the compared runs exercised.
#[derive(Default)]
pub(in crate::machine::sb) struct Tally {
    /// The oracle's counters (`DiffRun`): retired instructions, jumps, accesses, flag changes.
    pub run: DiffRun,
    /// Pairs of traces that were built, compared and run.
    pub pairs: u64,
    /// Fused uops by kind (`CMP_GUARD_I..=LDR_PAIR`) in the traces that ran.
    pub fused: [u64; 5],
    /// Runs that left through a fused guard / went past one.
    pub fused_taken: u64,
    pub fused_fell: u64,
    /// S operations and compares / `PRED`s the passes removed from the traces that ran.
    pub s_removed: u64,
    pub preds_removed: u64,
    /// Uops saved in all, over the traces that ran.
    pub uops_saved: u64,
    /// Runs in which the loop re-entered (budget above `n_guest`); L1: traces entered.
    pub laps: u64,
    pub entered: u64,
}

impl Tally {
    pub fn fused_total(&self) -> u64 {
        self.fused.iter().sum()
    }
}

/// An S operation or compare (a fused compare-and-guard still is one: the fusion removes no flag
/// definition): something `flagopt` may remove.
fn defines_flags(u: &Uop) -> bool {
    use crate::machine::sb::flagopt::effect::effect;
    matches!(u.kind, k::CMP_GUARD_I | k::CMP_GUARD_R | k::TST_GUARD_I)
        || (k::family(u.kind) == k::Family::Dp && effect(u).writes != 0)
}

fn count(sb: &Sb, f: impl Fn(&Uop) -> bool) -> u64 {
    sb.uops.iter().filter(|u| f(u)).count() as u64
}

/// The guest instructions a trace covers up to and including each uop that leaves it equal the
/// `done` of its exit (L6 on `Uop::n`, with the fused guards `testexec::check_sums` lacks).
fn sums_ok(sb: &Sb) -> Result<(), String> {
    let mut sum = 0u64;
    for (i, u) in sb.uops.iter().enumerate() {
        sum += u64::from(u.n);
        let leaves = matches!(
            u.kind,
            k::GUARD
                | k::GUARD_REG_EQ
                | k::LOOP_BACK
                | k::EXIT
                | k::EXIT_REG
                | k::EXIT_REG_ARM
                | k::GENERIC
                | k::GENERIC_END
                | k::CMP_GUARD_I
                | k::CMP_GUARD_R
                | k::TST_GUARD_I
        ) || k::is_store(u.kind);
        if leaves && sum != u64::from(sb.exits[u.exit_idx()].done) {
            return Err(format!("uop {i} (kind {}): sum of n {sum} != done", u.kind));
        }
        if u.kind == k::PRED && i + 1 + u.imm as usize > sb.uops.len() {
            return Err(format!("uop {i}: the predicate body runs past the trace"));
        }
    }
    Ok(())
}

fn same_exits(a: &Sb, b: &Sb) -> Result<(), String> {
    if (a.entry, a.n_guest, &a.words, a.exits.len())
        != (b.entry, b.n_guest, &b.words, b.exits.len())
    {
        return Err("entry, n_guest, words or the number of exits differ".into());
    }
    for (i, (x, y)) in a.exits.iter().zip(b.exits.iter()).enumerate() {
        let (x, y) = (
            (x.kind, x.reg, x.done, x.target, x.last_pc),
            (y.kind, y.reg, y.done, y.target, y.last_pc),
        );
        if x != y {
            return Err(format!("exit {i}: {x:x?} != {y:x?}"));
        }
    }
    Ok(())
}

/// Run `sb` from `cpu` on a fresh logging bus holding `words`. `stale`: 0 no store makes code
/// stale, 1 a store does, 2 stale from the start (I6 early exits).
fn execute(sb: &Sb, cpu: &Cpu, words: &[u32], fill: u32, left: u64, stale: u64) -> Outcome {
    let (mut cpu, mut bus, mut left) = (cpu.clone(), bus_with(words, fill), left);
    bus.stale_range = (stale == 1).then_some((0, u32::MAX));
    bus.stale = stale == 2;
    let out = run_sb(&mut cpu, &mut bus, sb, &mut left, false);
    let fault = cpu.fault.take();
    Outcome {
        cpu,
        fault,
        bus,
        exit: out.exit,
        left,
    }
}

struct Outcome {
    cpu: Cpu,
    fault: Option<Fault>,
    bus: LogBus,
    exit: u16,
    left: u64,
}

fn bus_with(words: &[u32], fill: u32) -> LogBus {
    let mut bus = LogBus::new(fill);
    bus.load_words(CODE, words);
    bus
}

fn same_state(
    what: &str,
    want: (&Cpu, Option<Fault>, &LogBus),
    got: &Outcome,
) -> Result<(), String> {
    if let Some(d) = CpuSnap::of(want.0, want.1).diff(&CpuSnap::of(&got.cpu, got.fault)) {
        return Err(format!("{what}: {d}"));
    }
    if let Some(d) = want.2.diff(&got.bus) {
        return Err(format!("{what}: access logs differ: {d}"));
    }
    if want.2.pc != got.bus.pc {
        return Err(format!(
            "{what}: bus.pc {:#x} != {:#x}",
            want.2.pc, got.bus.pc
        ));
    }
    Ok(())
}

/// One L0 case: the trace of `words` at word `start`, built without and with `passes`, run from
/// a random state with `extra` instructions of budget above `n_guest`; `Err` names the first
/// difference.
pub(in crate::machine::sb) fn check_pair(
    words: &[u32],
    start: usize,
    seed: u64,
    extra: u64,
    passes: Passes,
    tally: &mut Tally,
) -> Result<(), String> {
    let mut mem = Mem::new();
    for (i, w) in words.iter().enumerate() {
        mem.write32(CODE + 4 * i as u32, *w);
    }
    let pc = CODE + 4 * start as u32;
    let Some(base) = build(&mut mem, pc, &Passes::NONE.opts()) else {
        return Ok(());
    };
    let Some(sb) = build(&mut mem, pc, &passes.opts()) else {
        return Err("the passes made a buildable trace unbuildable".into());
    };
    same_exits(&base, &sb)?;
    sums_ok(&base)?;
    sums_ok(&sb)?;
    if sb.uops.len() > base.uops.len() {
        return Err("the passes made the trace longer".into());
    }

    let mut rng = Rng::new(seed ^ 0xFEED);
    let nzcv = rng.below(16) as u8;
    let cpu = random_cpu_flags(&mut rng, pc, nzcv);
    let fill = seed as u32;
    let left = u64::from(base.n_guest) + extra;
    // the dead-flag pass is the one whose decisions depend on early exits: it gets them often
    let mix: &[u64] = if passes.dead_flags {
        &[0, 1, 2]
    } else {
        &[0, 0, 0, 0, 0, 0, 1, 2]
    };
    let stale = mix[seed as usize % mix.len()];
    let a = execute(&base, &cpu, words, fill, left, stale);
    let b = execute(&sb, &cpu, words, fill, left, stale);
    let retired = left - b.left;
    let oracle: TraceRun = oracle_trace_on(&cpu, bus_with(words, fill), retired);
    let want = (&oracle.cpu, oracle.fault, &oracle.bus);
    same_state("unpassed trace against the interpreter", want, &a)?;
    same_state("passed trace against the interpreter", want, &b)?;
    if (a.exit, a.left) != (b.exit, b.left) {
        return Err(format!(
            "exit {} left {} != exit {} left {}",
            a.exit, a.left, b.exit, b.left
        ));
    }

    tally.run.note(&cpu, &oracle);
    tally.pairs += 1;
    tally.laps += u64::from(retired > u64::from(sb.exits[b.exit as usize].done));
    tally.add_fused(&sb);
    let guards = |u: &Uop| matches!(u.kind, k::CMP_GUARD_I | k::CMP_GUARD_R | k::TST_GUARD_I);
    let took = sb
        .uops
        .iter()
        .any(|u| guards(u) && u.exit_idx() == b.exit as usize);
    tally.fused_taken += u64::from(took);
    tally.fused_fell += u64::from(!took && count(&sb, guards) > 0);
    tally.s_removed += count(&base, defines_flags).saturating_sub(count(&sb, defines_flags));
    let preds = |u: &Uop| u.kind == k::PRED;
    tally.preds_removed += count(&base, preds).saturating_sub(count(&sb, preds));
    tally.uops_saved += (base.uops.len() - sb.uops.len()) as u64;
    Ok(())
}

/// `testgen::drive_traces` with the caller's options: at every pc build a trace and run it when
/// the driver contract allows (budget covers `n_guest`, pages marked as code, stale queue
/// drained), otherwise take one step of the real `Machine::run`.
fn drive_opts(m: &mut Machine, max: u64, opts: &BuildOpts, tally: &mut Tally) -> (u64, Stop) {
    let start = m.cpu.insns;
    let limit = start + max;
    let reason = loop {
        if m.cpu.insns >= limit {
            break Stop::Budget;
        }
        let pc = m.cpu.r[15];
        m.drop_stale_code();
        let left = limit - m.cpu.insns;
        let sb = if m.cpu.thumb || m.bus.mem.flags(pc) & HOOK_X != 0 {
            None
        } else {
            build(&mut m.bus.mem, pc, opts).filter(|sb| u64::from(sb.n_guest) <= left)
        };
        match sb {
            Some(sb) => {
                for page in &sb.pages[..sb.n_pages as usize] {
                    m.bus.mem.mark_code(page << 12);
                }
                tally.entered += 1;
                tally.add_fused(&sb);
                let mut left = left;
                run_sb(&mut m.cpu, &mut m.bus, &sb, &mut left, false);
                m.drop_stale_code();
                if let Some(f) = m.cpu.fault.take() {
                    break Stop::Fault(f);
                }
            }
            None => match m.run(1, 0, None) {
                (_, Stop::Budget) => {}
                (_, other) => break other,
            },
        }
    };
    (m.cpu.insns - start, reason)
}

impl Tally {
    fn add_fused(&mut self, sb: &Sb) {
        for u in sb
            .uops
            .iter()
            .filter(|u| k::family(u.kind) == k::Family::Fused)
        {
            self.fused[(u.kind - k::CMP_GUARD_I) as usize] += 1;
        }
    }
}

/// One L1 case: `budget` instructions of `words` on the interpreter and on traces built with
/// `passes`, compared on the whole machine.
pub(in crate::machine::sb) fn program_check(
    words: &[u32],
    seed: u64,
    budget: u64,
    passes: Passes,
    tally: &mut Tally,
) -> Result<(), String> {
    let m = machine_with(words, seed, true);
    let want = oracle_run(&m, budget, 0, None);
    let mut got = m;
    let res = drive_opts(&mut got, budget, &passes.opts(), tally);
    compare_machines(&want.m, (want.n, want.why), &got, res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::fuse::gen::program;

    #[test]
    fn a_pair_check_compares_exits_states_and_logs_and_fills_the_tally() {
        let words = program(&mut Rng::new(11), 80);
        let mut tally = Tally::default();
        for start in 0..30 {
            check_pair(&words, start, 5 + start as u64, 7, Passes::ALL, &mut tally).unwrap();
        }
        assert!(
            tally.pairs > 20 && tally.run.retired > 100,
            "{}",
            tally.pairs
        );
        assert!(tally.uops_saved > 0, "the passes shortened some trace");
    }

    #[test]
    fn a_program_check_runs_traces_against_the_interpreter() {
        let words = program(&mut Rng::new(12), 80);
        let mut tally = Tally::default();
        for budget in [1, 7, 40, 300] {
            program_check(&words, 3, budget, Passes::ALL, &mut tally).unwrap();
        }
        assert!(tally.entered > 0);
    }

    #[test]
    fn the_checks_notice_a_trace_that_differs_from_the_unpassed_one() {
        // a failing comparison is reported, not swallowed: perturb the unpassed run's budget
        let words = program(&mut Rng::new(13), 60);
        let (mut mem, pc) = (Mem::new(), CODE);
        for (i, w) in words.iter().enumerate() {
            mem.write32(CODE + 4 * i as u32, *w);
        }
        let sb = build(&mut mem, pc, &Passes::NONE.opts()).unwrap();
        let cpu = random_cpu_flags(&mut Rng::new(1), pc, 3);
        let a = execute(&sb, &cpu, &words, 1, u64::from(sb.n_guest), 0);
        let mut b = execute(&sb, &cpu, &words, 1, u64::from(sb.n_guest), 0);
        b.cpu.r[3] ^= 1;
        let want = (&a.cpu, a.fault, &a.bus);
        assert!(same_state("x", want, &b).unwrap_err().contains("r3"));
        b.cpu.r[3] ^= 1;
        b.bus.log.pop();
        b.bus.log.push((9, 0, 0, 0));
        assert!(same_state("x", want, &b).is_err());
    }
}
