//! OWNER: W1-VGEN. The oracle runner: T0, `Cpu::step` without any cache, on a clone, and the
//! comparison of any other run against it.
//!
//! Spec (design note 8): "Oracle: `Cpu::step` with `predecode = false` on a `SysBus`". Two
//! shapes: [`oracle_run`] runs a whole [`Machine`] (hooks, `until`, `stop`, budget: the outer
//! loop every tier must reproduce) and [`oracle_trace`] runs one to three instructions on a
//! [`LogBus`] (L0: registers, flags, ordered access log, `bus.pc`).

use super::logbus::LogBus;
use super::snapshot::{mem_diff, snapshot_run, Snapshot};
use crate::cpu::{Cpu, Fault};
use crate::machine::{Machine, Stop};

/// What a run of a machine left behind: the machine, the instructions it retired and why it
/// stopped. Built by [`oracle_run`] or [`Run::of`] from any other tier.
pub(crate) struct Run {
    pub m: Machine,
    pub n: u64,
    pub why: Stop,
}

impl Run {
    /// Wrap the result of `Machine::run` (or of a tier driver) on `m`.
    pub fn of(m: Machine, (n, why): (u64, Stop)) -> Self {
        Self { m, n, why }
    }

    /// The full observable state, the stop reason included.
    pub fn snapshot(&self) -> Snapshot {
        snapshot_run(&self.m, &self.why)
    }
}

/// The oracle: a clone of `m` with `predecode = false` (the plain fetch-decode-execute path of
/// `Machine::run`, `Cpu::step` per instruction), run for `budget` instructions. `m` is untouched.
pub(crate) fn oracle_run(m: &Machine, budget: u64, until: u32, stop_pc: Option<u32>) -> Run {
    let mut o = m.clone();
    o.predecode = false;
    let res = o.run(budget, until, stop_pc);
    Run::of(o, res)
}

/// Compare `got` against the oracle run `want`: the instruction count and stop reason, then the
/// whole snapshot. `Err` is a readable line; the left value of every `!=` is the oracle's, and a
/// memory mismatch names the first differing words.
pub(crate) fn compare_runs(want: &Run, got: &Run) -> Result<(), String> {
    compare_machines(&want.m, (want.n, want.why), &got.m, (got.n, got.why))
}

/// [`compare_runs`] on machines and results that are not wrapped in a [`Run`] (a machine that
/// goes on running after the comparison, as in a schedule of slices).
pub(crate) fn compare_machines(
    want: &Machine,
    (wn, why): (u64, Stop),
    got: &Machine,
    (gn, gwhy): (u64, Stop),
) -> Result<(), String> {
    if (wn, why) != (gn, gwhy) {
        return Err(format!(
            "ran {gn} insns and stopped with {gwhy:?}; the oracle ran {wn} and {why:?}"
        ));
    }
    let Some(mut d) = snapshot_run(want, &why).diff(&snapshot_run(got, &gwhy)) else {
        return Ok(());
    };
    if d.contains("mem_hash") {
        let words = mem_diff(&want.bus.mem, &got.bus.mem, 4);
        let shown: Vec<String> = words
            .iter()
            .map(|(a, x, y)| format!("[{a:#x}] {x:#x} != {y:#x}"))
            .collect();
        d.push_str(&format!("; differing words: {}", shown.join(", ")));
    }
    Err(d)
}

/// What [`oracle_trace`] left behind.
pub(crate) struct TraceRun {
    pub cpu: Cpu,
    pub bus: LogBus,
    /// Instructions started, the faulting one included.
    pub executed: u64,
    pub fault: Option<Fault>,
}

/// Run `cpu` (a clone) on `bus` with `Cpu::step` for at most `steps` instructions, setting
/// `bus.pc` to the address of each before it runs, and stop after the first fault.
pub(crate) fn oracle_trace_on(cpu: &Cpu, mut bus: LogBus, steps: u64) -> TraceRun {
    let mut cpu = cpu.clone();
    let (mut executed, mut fault) = (0, None);
    while executed < steps {
        bus.pc = cpu.r[15];
        executed += 1;
        if let Err(f) = cpu.step(&mut bus) {
            fault = Some(f);
            break;
        }
    }
    TraceRun {
        cpu,
        bus,
        executed,
        fault,
    }
}

/// [`oracle_trace_on`] a fresh [`LogBus`] filled with `seed` holding `words` at `cpu.r[15]`.
pub(crate) fn oracle_trace(cpu: &Cpu, words: &[u32], seed: u64, steps: u64) -> TraceRun {
    let mut bus = LogBus::new(seed as u32);
    bus.load_words(cpu.r[15], words);
    oracle_trace_on(cpu, bus, steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testkit::{random_cpu, Rng};

    const CODE: u32 = 0xC000_0000;

    fn machine(code: &[u32]) -> Machine {
        let mut m = Machine::new();
        for (i, w) in code.iter().enumerate() {
            m.bus.mem.write32(CODE + 4 * i as u32, *w);
        }
        m.cpu.r[15] = CODE;
        m
    }

    #[test]
    fn the_oracle_runs_a_clone_on_the_plain_interpreter() {
        // mov r0,#5 ; str r0,[r8] ; b .
        let mut m = machine(&[0xE3A0_0005, 0xE588_0000, 0xEAFF_FFFE]);
        m.cpu.r[8] = 0x9000_0000;
        m.predecode = true;
        let before = m.state_hash();
        let run = oracle_run(&m, 10, 0, None);
        assert_eq!((run.n, run.why), (10, Stop::Budget));
        assert_eq!(
            (run.m.cpu.r[0], run.m.bus.mem.clone().read32(0x9000_0000)),
            (5, 5)
        );
        assert!(!run.m.predecode, "T0: no predecode");
        assert_eq!(run.m.code.live_pages(), 0, "and nothing was decoded");
        assert!(
            m.predecode && m.cpu.insns == 0 && m.state_hash() == before,
            "m is untouched"
        );
        let to = oracle_run(&m, 10, CODE + 8, None);
        assert_eq!((to.n, to.why), (2, Stop::Until));
        let st = oracle_run(&m, 10, 0, Some(CODE + 4));
        assert_eq!((st.n, st.why), (1, Stop::StopPc));
    }

    #[test]
    fn compare_runs_accepts_the_same_run_and_reports_every_kind_of_divergence() {
        let mut m = machine(&[0xE3A0_0005, 0xE588_0000, 0xEAFF_FFFE]);
        m.cpu.r[8] = 0x9000_0000;
        let want = oracle_run(&m, 6, 0, None);
        let mut t1 = m.clone();
        t1.predecode = true;
        let res = t1.run(6, 0, None);
        assert_eq!(compare_runs(&want, &Run::of(t1, res)), Ok(()));
        let short = oracle_run(&m, 5, 0, None);
        let e = compare_runs(&want, &short).unwrap_err();
        assert!(
            e.contains("ran 5 insns") && e.contains("the oracle ran 6"),
            "{e}"
        );
        let mut reg = Run::of(want.m.clone(), (want.n, want.why));
        reg.m.cpu.r[4] = 1;
        let e = compare_runs(&want, &reg).unwrap_err();
        assert!(e.contains("r4: 0x0 != 0x1"), "{e}");
        let mut mem = Run::of(want.m.clone(), (want.n, want.why));
        mem.m.bus.mem.write32(0x9000_0010, 0x77);
        let e = compare_runs(&want, &mem).unwrap_err();
        assert!(
            e.contains("mem_hash") && e.contains("[0x90000010] 0x0 != 0x77"),
            "{e}"
        );
        let other_stop = oracle_run(&m, 6, 0, Some(CODE));
        assert!(compare_runs(&want, &other_stop).is_err());
    }

    #[test]
    fn a_fault_is_part_of_the_outcome_and_counts_the_faulting_instruction() {
        let m = machine(&[0xE3A0_0005, 0xE7F0_00F0, 0xE3A0_0006]);
        let run = oracle_run(&m, 10, 0, None);
        assert_eq!(
            run.why,
            Stop::Fault(Fault::Undefined {
                pc: CODE + 4,
                insn: 0xE7F0_00F0
            })
        );
        assert_eq!(run.n, 2, "the undefined word is retired and counted");
        assert_eq!(
            run.snapshot().cpu.fault,
            Some(Fault::Undefined {
                pc: CODE + 4,
                insn: 0xE7F0_00F0
            })
        );
    }

    #[test]
    fn the_trace_oracle_logs_the_accesses_with_the_pc_of_each_instruction() {
        let mut cpu = random_cpu(&mut Rng::new(1), 0x1000);
        cpu.r[1] = 0x2000;
        cpu.set_cpsr(0x13);
        // mov r0,#9 ; str r0,[r1] ; ldr r2,[r1] ; add r0,r0,#1
        let words = [0xE3A0_0009, 0xE581_0000, 0xE591_2000, 0xE280_0001];
        let t = oracle_trace(&cpu, &words, 3, 3);
        assert_eq!(
            (t.executed, t.fault, t.cpu.r[0], t.cpu.r[2]),
            (3, None, 9, 9)
        );
        assert_eq!(t.cpu.r[15], 0x100C);
        assert_eq!(
            t.bus.log,
            vec![(5, 0x2000, 9, 0x1004), (2, 0x2000, 0, 0x1008)]
        );
        assert_eq!((t.bus.loads(), t.bus.stores(), t.bus.pc), (1, 1, 0x1008));
        assert_eq!(cpu.r[15], 0x1000, "the input CPU is not advanced");
    }

    #[test]
    fn the_trace_oracle_stops_at_the_first_fault_and_counts_it() {
        let cpu = random_cpu(&mut Rng::new(2), 0x1000);
        let t = oracle_trace(&cpu, &[0xE7F0_00F0, 0xE3A0_0001], 1, 3);
        assert_eq!(t.executed, 1);
        assert_eq!(
            t.fault,
            Some(Fault::Undefined {
                pc: 0x1000,
                insn: 0xE7F0_00F0
            })
        );
        let none = oracle_trace(&cpu, &[0xE3A0_0001], 1, 0);
        assert_eq!((none.executed, none.cpu.r[15]), (0, 0x1000));
        let mut bus = LogBus::new(1);
        bus.load_words(0x1000, &[0xE581_0000]); // str r0,[r1]
        bus.stale_range = Some((0x2000, 0x3000));
        let mut cpu = cpu;
        cpu.r[1] = 0x2000;
        assert!(
            oracle_trace_on(&cpu, bus, 1).bus.stale,
            "a store into the range marks it stale"
        );
    }
}
