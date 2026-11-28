//! OWNER: W1-VGEN (scaffold written by S0; W1-VGEN extends it and keeps every signature).
//! Generators and the "op through lowering versus interpreter" helpers every work package calls
//! from its own tests.
//!
//! Spec (design note 8, WP W1-VGEN): `cfg(test)` support only.
//!
//! * [`random_program`], [`random_word`], [`tiny_loop`] (`testgen/mixed.rs`): the scaffold's flat
//!   generators, streams unchanged.
//! * [`structured_program`] (`testgen/program.rs`, constructs in `blocks.rs` and `parts.rs`): the
//!   L1 generator of design note 8: loops, `bl`/`bx lr` pairs, conditional runs, literal loads,
//!   stores into the own page, the next word and a literal the code loaded, device accesses,
//!   Thumb entry, and words that fault (undefined, `swi`, `wfi`: `testgen/faults.rs`, as a body
//!   construct, inside a callee, or as a patch word a store copies over the program). A
//!   [`Program`] becomes a machine with [`machine_from`].
//! * [`random_of`] / [`Family`] (`testgen/words.rs`): one word of each instruction family, with
//!   the shift amounts, addressing modes and register aliasing of design note 8, L0.
//! * [`Schedule`] / [`schedule_vs_oracle`] (`testgen/schedule.rs`): slices with budgets,
//!   `until`/`stop_pc` addresses and hooks added between them, compared with the oracle slice
//!   by slice.
//! * [`budget_sweep`] / [`sweep_vs_oracle`] (`testgen/sweep.rs`): every budget of a range as a
//!   two-slice schedule, so the budget expires at every instruction position (design note 8:
//!   "budgets 1..7 and 1..400").
//! * [`asm`]: the encoders the generators build on.
//! * [`lowered_vs_step`] / [`uops_vs_step`]: one instruction (or a hand-built trace for it)
//!   through the lowering and the T2 executor against `Cpu::step`, with registers, flags, fault,
//!   the ordered bus access log and `bus.pc` compared ([`compare_outcome`], itself tested part by
//!   part in `testgen/comparator.rs`). This is the helper of the lowering and
//!   executor work packages (W1-DP, W1-MEM, W1-XALU, W1-XMEM, W2-XCORE) and of L0 in `difftest`.
//!   The `_run` forms also count what the oracle did into a `DiffRun`.
//! * [`drive_traces`] (`testgen/drive.rs`): the tier stack without a cache, as a copy of the outer
//!   loop of `Machine::run` that builds a trace per pc and runs it under the driver's side of the
//!   contract (I7 entry test, code pages marked, stale drain): the proof that the always-correct
//!   baseline equals the interpreter.
//!
//! `#![allow(dead_code)]`: consumed by the tests of the work packages named above.

#![allow(dead_code)]

pub(crate) mod asm;
mod blocks;
#[cfg(test)]
mod comparator;
#[cfg(test)]
mod coverage;
mod drive;
mod faults;
mod mixed;
mod parts;
pub(crate) mod program;
pub(crate) mod schedule;
pub(crate) mod sweep;
pub(crate) mod words;

use super::config::SbConfig;
use super::decode::{decode, lower_or_generic, LowerCtx};
use super::exec::run_sb;
use super::lower_flow::{lower_flow, Flow, FlowCtx};
use super::testkit::logbus::kind;
use super::testkit::oracle::oracle_trace_on;
use super::testkit::{CpuSnap, DiffRun, LogBus, TraceRun};
use super::trace::BuildOpts;
use super::uop::{k, Exit, ExitKind, Sb, Uop};
use crate::cpu::{Cpu, Fault};
use crate::machine::Machine;

pub(crate) use drive::drive_traces;
pub(crate) use mixed::{random_program, random_word, tiny_loop};
pub(crate) use program::{machine_from, structured_program, Program};
pub(crate) use schedule::{random_schedule, schedule_vs_oracle, SchedStats, Schedule};
pub(crate) use sweep::{budget_sweep, sweep_vs_oracle};
pub(crate) use words::{random_of, Family};

/// Base address of generated programs.
pub(crate) const CODE: u32 = 0xC000_0000;

/// A machine with `words` at [`CODE`], random `r0..r7` from `seed`, `r8`/`r9` pointing at data,
/// `r10` into the code itself, a stack, and `pc = CODE`; `predecode` selects tier 1 or the plain
/// interpreter. (The flat image of [`Program::flat`].)
pub(crate) fn machine_with(words: &[u32], seed: u64, predecode: bool) -> Machine {
    machine_from(&Program::flat(words, seed), predecode)
}

/// Lower the one instruction `word` at `pc` into a complete trace, the way the trace builder
/// does for a one-instruction trace: control flow through `lower_flow`, everything else through
/// the static registry with the early exits allocated. `peek` serves literal folding; like the
/// builder, the trace lists each folded literal word in `Sb::words` after the instruction itself
/// (I1: the trace depends on it), which is how [`uops_vs_step_run`] knows the load it replaced.
pub(crate) fn lower_single(word: u32, pc: u32, peek: &dyn Fn(u32) -> u32) -> Sb {
    let insn = decode(pc, word);
    let opts = BuildOpts::from(&SbConfig::on());
    let (mut uops, mut exits): (Vec<Uop>, Vec<Exit>) = (Vec::new(), Vec::new());
    let mut words = vec![(pc, word)];
    let next = if insn.is_ender() {
        let (mut depth, mut rets) = (0u8, Vec::new());
        let can_follow = |_: u32| false;
        match lower_flow(&mut FlowCtx {
            insn,
            uops: &mut uops,
            exits: &mut exits,
            done: 0,
            taken_hint: None,
            can_follow: &can_follow,
            depth: &mut depth,
            rets: &mut rets,
            entry_pc: pc,
            opts: &opts,
        }) {
            Flow::End => None,
            Flow::Follow(next) => Some(next),
        }
    } else {
        let mut fold = |a: u32| {
            let literal = peek(a);
            if !words.contains(&(a, literal)) {
                words.push((a, literal));
            }
            Some(literal)
        };
        lower_or_generic(&mut LowerCtx {
            insn,
            out: &mut uops,
            fold: &mut fold,
            opts: &opts,
        });
        for u in uops.iter_mut() {
            if k::is_store(u.kind) || u.kind == k::GENERIC {
                exits.push(Exit::new(ExitKind::Stale, 1, pc, pc.wrapping_add(4)));
                u.set_exit_idx(exits.len() as u8 - 1);
            }
        }
        Some(pc.wrapping_add(4))
    };
    if let Some(next) = next {
        exits.push(Exit::new(ExitKind::Direct, 1, pc, next));
        uops.push(Uop::exit(exits.len() as u8 - 1, pc));
    }
    Sb::from_parts(pc, 1, uops, exits, words).expect("a one-instruction trace fits")
}

/// Run `sb` (a trace of the one instruction `word` at `cpu.r[15]`) and `Cpu::step` on equal
/// states; `Err` names the first difference in registers, flags, fault, bus access log or `pc`.
pub(crate) fn uops_vs_step(word: u32, sb: &Sb, cpu: &Cpu, seed: u64) -> Result<(), String> {
    uops_vs_step_run(word, sb, cpu, seed, &mut DiffRun::default())
}

/// [`uops_vs_step`] that also counts what the oracle did into `run` (even when the comparison
/// fails), so the test can end with `run.assert_exercised(..)`.
pub(crate) fn uops_vs_step_run(
    word: u32,
    sb: &Sb,
    cpu: &Cpu,
    seed: u64,
    run: &mut DiffRun,
) -> Result<(), String> {
    let pc = cpu.r[15];
    let mut ba = LogBus::new(seed as u32);
    ba.poke32(pc, word);
    let t = oracle_trace_on(cpu, ba, 1);
    run.note(cpu, &t);
    let (mut cb, mut bb) = (cpu.clone(), LogBus::new(seed as u32));
    bb.poke32(pc, word);
    // `watch`: a trace that loops back to its head (`b .`) leaves after one lap instead of
    // re-entering while the budget of 4 lasts, so one instruction retires on both sides.
    let mut left = 4u64;
    run_sb(&mut cb, &mut bb, sb, &mut left, true);
    let rb = cb.fault.take();
    let t = without_folded_loads(t, sb);
    compare_outcome(word, &t, &cb, rb, &bb, left)
}

/// The oracle's run `t` without the loads the trace `sb` replaced by a constant: a literal load
/// folds to `MOV_IMM` (design note 4.2), so the trace performs no bus access for it, while the
/// interpreter reads the word. The folded words are `sb.words` after the instruction itself
/// ([`lower_single`]); a load is dropped when its word is one of them. Everything else stays in
/// the log, so a trace that skips a real access is still caught.
fn without_folded_loads(mut t: TraceRun, sb: &Sb) -> TraceRun {
    let folded: Vec<u32> = sb.words.iter().skip(1).map(|w| w.0 & !3).collect();
    if !folded.is_empty() {
        t.bus
            .log
            .retain(|e| kind::is_store(e.0) || !folded.contains(&(e.1 & !3)));
    }
    t
}

/// The comparison of [`uops_vs_step_run`], on what the executor left behind: its CPU `cb` and
/// fault `rb`, its bus `bb` and the budget `left` of the 4 instructions it was given, against the
/// oracle's trace `t` of the one instruction `word`. `Err` names the first difference: the CPU
/// state, then the ordered access log (kind, address, value and `pc` of every access), then
/// `bus.pc` and `left` (one instruction retired). Separate from the run so that a test can
/// perturb each part of an outcome on its own (`testgen/comparator.rs`).
pub(crate) fn compare_outcome(
    word: u32,
    t: &TraceRun,
    cb: &Cpu,
    rb: Option<Fault>,
    bb: &LogBus,
    left: u64,
) -> Result<(), String> {
    let pc = t.bus.pc;
    let (sa, sbn) = (CpuSnap::of(&t.cpu, t.fault), CpuSnap::of(cb, rb));
    if let Some(d) = sa.diff(&sbn) {
        return Err(format!("word {word:#010x} at {pc:#x}: {d}"));
    }
    if let Some(d) = t.bus.diff(bb) {
        return Err(format!(
            "word {word:#010x} at {pc:#x}: access logs differ: {d}"
        ));
    }
    if (t.bus.pc, left) != (bb.pc, 3) {
        return Err(format!(
            "word {word:#010x} at {pc:#x}: bus.pc {:#x} vs {:#x}, left {left}",
            t.bus.pc, bb.pc
        ));
    }
    Ok(())
}

/// The lowering of `word` at `cpu.r[15]` against the interpreter ([`lower_single`] then
/// [`uops_vs_step`]). The helper of every lowering work package.
pub(crate) fn lowered_vs_step(word: u32, cpu: &Cpu, seed: u64) -> Result<(), String> {
    lowered_vs_step_run(word, cpu, seed, &mut DiffRun::default())
}

/// [`lowered_vs_step`] counting what the oracle did into `run`.
pub(crate) fn lowered_vs_step_run(
    word: u32,
    cpu: &Cpu,
    seed: u64,
    run: &mut DiffRun,
) -> Result<(), String> {
    let mut probe = LogBus::new(seed as u32);
    probe.poke32(cpu.r[15], word); // the memory the oracle runs on: a literal may be the word itself
    let sb = lower_single(word, cpu.r[15], &|a| probe.peek32(a));
    uops_vs_step_run(word, &sb, cpu, seed, run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testkit::{random_cpu, Need, Rng};

    #[test]
    fn a_machine_starts_at_the_code_with_data_pointers_and_is_reproducible() {
        let words = random_program(&mut Rng::new(9), 64);
        let (a, b) = (machine_with(&words, 5, true), machine_with(&words, 5, true));
        assert_eq!(
            (a.cpu.r[15], a.cpu.r[8], a.bus.mem.clone().read32(CODE + 8)),
            (CODE, 0x9000_0000, words[2])
        );
        assert_eq!(a.cpu.r, b.cpu.r);
        assert_ne!(a.cpu.r[0], machine_with(&words, 6, true).cpu.r[0]);
    }

    #[test]
    fn the_skeleton_lowering_equals_the_interpreter_on_random_words_and_states() {
        let mut rng = Rng::new(0xA11CE);
        let mut run = DiffRun::default();
        for i in 0..3000 {
            let word = random_word(&mut rng, 40, 400);
            let word = if i % 3 == 0 {
                word & 0x0FFF_FFFF | 0xE000_0000
            } else {
                word
            };
            let cpu = random_cpu(&mut rng, 0x1000);
            lowered_vs_step_run(word, &cpu, i, &mut run).unwrap();
        }
        run.assert_exercised(
            "skeleton lowering on random words",
            &Need {
                cases: 3000,
                retired: 3000,
                loads: 150,
                stores: 150,
                faults: 50,
                jumps: 100,
                reg_writes: 1500,
                flag_changes: 300,
                max_idle_pct: 20,
            },
        );
    }

    #[test]
    fn the_skeleton_lowering_equals_the_interpreter_on_every_instruction_family() {
        let mut rng = Rng::new(0xFA11);
        let mut run = DiffRun::default();
        for family in Family::ALL {
            for i in 0..150 {
                let word = random_of(&mut rng, family);
                let cpu = random_cpu(&mut rng, 0x1000);
                lowered_vs_step_run(word, &cpu, i, &mut run)
                    .unwrap_or_else(|e| panic!("{family:?}: {e}"));
            }
        }
        run.assert_exercised(
            "skeleton lowering on every family",
            &Need {
                cases: 150 * Family::ALL.len() as u64,
                loads: 500,
                stores: 500,
                faults: 100,
                jumps: 200,
                reg_writes: 1500,
                flag_changes: 300,
                max_idle_pct: 30,
                ..Need::default()
            },
        );
    }

    #[test]
    fn the_comparator_catches_a_trace_that_does_something_else() {
        let mut rng = Rng::new(5);
        let cpu = random_cpu(&mut rng, 0x1000);
        let wrong = lower_single(0xE3A0_0006, 0x1000, &|_| 0); // mov r0,#6 ...
        let err = uops_vs_step(0xE3A0_0005, &wrong, &cpu, 1).unwrap_err(); // ... compared to mov r0,#5
        assert!(err.contains("r0"), "{err}");
        let nothing = Sb::from_parts(
            0x1000,
            1,
            vec![Uop::exit(0, 0x1000)],
            vec![Exit::new(ExitKind::Direct, 1, 0x1000, 0x1004)],
            vec![(0x1000, 0xE3A0_0005)],
        )
        .unwrap();
        assert!(uops_vs_step(0xE3A0_0005, &nothing, &cpu, 1).is_err());
    }

    #[test]
    fn a_comparison_that_fails_is_still_counted_and_an_idle_one_is_rejected() {
        let cpu = random_cpu(&mut Rng::new(5), 0x1000);
        let wrong = lower_single(0xE3A0_0006, 0x1000, &|_| 0);
        let mut run = DiffRun::default();
        assert!(uops_vs_step_run(0xE3A0_0005, &wrong, &cpu, 1, &mut run).is_err());
        assert_eq!((run.cases, run.retired), (1, 1), "the oracle side counted");
        let mut idle = DiffRun::default();
        let mut c = cpu.clone();
        c.set_cpsr(c.cpsr() & 0x0FFF_FFFF); // all flags clear: `moveq` does nothing
        lowered_vs_step_run(0x03A0_0001, &c, 1, &mut idle).unwrap();
        assert_eq!(idle.idle, 1);
        assert!(idle
            .require(&Need {
                max_idle_pct: 50,
                ..Need::default()
            })
            .is_err());
    }

    #[test]
    fn a_conditional_and_a_branch_lower_to_something_that_runs() {
        let mut rng = Rng::new(6);
        let cpu = random_cpu(&mut rng, 0x1000);
        for word in [
            0x1280_0001,
            0xEAFF_FFFE,
            0xE12F_FF10,
            0xE591_F000,
            0xE582_1000,
        ] {
            lowered_vs_step(word, &cpu, 3).unwrap();
        }
    }

    #[test]
    fn the_hub_reexports_the_generators() {
        let p = structured_program(&mut Rng::new(1), 30);
        assert_eq!(machine_from(&p, true).cpu.r[15], CODE);
        let m = machine_with(&tiny_loop(), 1, false);
        let s = random_schedule(&mut Rng::new(2), &Program::flat(&tiny_loop(), 1), 3);
        let run = schedule_vs_oracle(&m, &s, &mut |m, n, u, st| drive_traces(m, n, u, st)).unwrap();
        assert!(run.stats.slices >= 1 && run.stats.slices <= 3);
        let _: SchedStats = run.stats;
        let _: Schedule = s;
        assert_eq!(budget_sweep(1..=7, 9).len(), 7);
        let swept = sweep_vs_oracle(&m, 1..=3, 20, || {
            |m: &mut Machine, n, u, st| m.run(n, u, st)
        })
        .unwrap();
        assert_eq!(swept.slices, 6);
    }
}
