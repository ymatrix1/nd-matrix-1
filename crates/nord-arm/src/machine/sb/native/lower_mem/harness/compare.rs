//! OWNER: W2-NMEM. The comparison half of the native memory and flow tests: the state compare of
//! two runs ([`same_state`]) and the differential case ([`run_case`]) that runs native code and
//! the tier-2 oracle ([`super::oracle::walk`]) over the same world.

use super::oracle::{walk, Tally, Walk};
use crate::cpu::Cpu;
use crate::machine::sb::exec::run_sb;
use crate::machine::sb::native::rt::{Jit, Native};
use crate::machine::sb::testkit::CpuSnap;
use crate::machine::sb::uop::Sb;
use crate::machine::SysBus;
use crate::mem::Mem;

/// Compare registers, flags and counters, then the pages of the world and the mapping and stale
/// accounting of two memories.
pub(crate) fn same_state(
    label: &str,
    a: (&Cpu, &mut Mem),
    b: (&Cpu, &mut Mem),
) -> Result<(), String> {
    if let Some(d) = CpuSnap::of(a.0, None).diff(&CpuSnap::of(b.0, None)) {
        return Err(format!("{label}: cpu {d}"));
    }
    let (ma, mb) = (a.1, b.1);
    for page in 0..=super::PAGES + 1 {
        let base = super::page_addr(page);
        if ma.is_mapped(base) != mb.is_mapped(base) {
            return Err(format!("{label}: page {page} mapped differently"));
        }
        if !ma.is_mapped(base) {
            continue;
        }
        for w in 0..1024 {
            let at = base + 4 * w;
            let (x, y) = (ma.read32(at), mb.read32(at));
            if x != y {
                return Err(format!("{label}: [{at:#x}] {x:#x} != {y:#x}"));
            }
        }
    }
    if (
        ma.stale_code.clone(),
        ma.auto_mapped_count,
        ma.arena_bytes(),
    ) != (
        mb.stale_code.clone(),
        mb.auto_mapped_count,
        mb.arena_bytes(),
    ) {
        return Err(format!(
            "{label}: stale queue, auto-mapping or arena differ"
        ));
    }
    Ok(())
}

/// One input of a differential case.
#[derive(Clone)]
pub(crate) struct Case {
    pub cpu: Cpu,
    /// The seed of `world(seed)`.
    pub seed: u32,
    /// The instruction budget: `ctl.limit = insns + left`.
    pub left: u64,
    /// The idle-v2 request, which native code gets as a limit that never covers a back-edge.
    pub watch: bool,
}

impl Case {
    /// A case with the budget of a trace entered with plenty of room and no watch.
    pub fn new(cpu: Cpu, seed: u32) -> Self {
        Self {
            cpu,
            seed,
            left: 1000,
            watch: false,
        }
    }
}

/// One differential case: `native` on `case.cpu` over `world(case.seed)` against the oracle.
pub(crate) fn run_case(
    jit: &Jit,
    native: &Native,
    sb: &Sb,
    case: &Case,
    t: &mut Tally,
) -> Result<(), String> {
    let (cpu0, seed, left, watch) = (&case.cpu, case.seed, case.left, case.watch);
    let mut cn = cpu0.clone();
    let mut mem_n = super::world(seed);
    let limit = cpu0.insns + if watch { 0 } else { left };
    let out = jit.run(native, &mut cn, &mut mem_n, limit);
    let mut co = cpu0.clone();
    let mut bus = SysBus::new();
    bus.mem = super::world(seed);
    t.cases += 1;
    match walk(sb, &mut co, &mut bus, left, watch, t) {
        Walk::Slow { pc, retired, why } => {
            if !out.is_generic() || out.target != pc {
                return Err(format!(
                    "expected a generic exit at {pc:#x} ({why:?}), native exit {:#x} target {:#x}",
                    out.exit, out.target
                ));
            }
            co.insns = cpu0.insns + retired;
            co.r[15] = pc;
            t.slow_exits += 1;
            same_state("slow exit", (&cn, &mut mem_n), (&co, &mut bus.mem))
        }
        Walk::Complete => {
            let mut cw = cpu0.clone();
            let mut bw = SysBus::new();
            bw.mem = super::world(seed);
            let mut l = left;
            let t2 = run_sb(&mut cw, &mut bw, sb, &mut l, watch);
            if out.exit != u32::from(t2.exit) || out.target != t2.target {
                return Err(format!(
                    "exit {:#x}/{:#x} != tier 2 {}/{:#x}",
                    out.exit, out.target, t2.exit, t2.target
                ));
            }
            t.completed += 1;
            same_state("complete", (&cn, &mut mem_n), (&cw, &mut bw.mem))
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn a_state_compared_with_itself_is_the_same_and_a_changed_register_is_not() {
        let cpu = crate::cpu::Cpu::new();
        let mut m1 = super::super::world(1);
        let mut m2 = super::super::world(1);
        assert!(same_state("x", (&cpu, &mut m1), (&cpu, &mut m2)).is_ok());
        let mut other = cpu.clone();
        other.r[3] ^= 1;
        assert!(same_state("x", (&cpu, &mut m1), (&other, &mut m2)).is_err());
    }
}
