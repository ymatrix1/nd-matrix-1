//! OWNER: W2-NMEM. The oracle of the native memory and flow tests: tier 2 (`exec_flow`,
//! `exec_mem`, `exec_dp`, `exec::run_sb`) on a `SysBus` over the same world, a classification of
//! every memory uop into "the fast path takes it" or "native code leaves for the interpreter",
//! and the comparison of a native run with the oracle.
//!
//! # The contract compared
//!
//! A native run either completes the trace or leaves with `EXIT_GENERIC` before the first uop
//! whose accesses are not all on mapped pages without the blocking hook flag in one page (design
//! note 9). [`walk`] runs the trace on tier 2 up to that uop (stopping *before* it) and says
//! which case applies:
//!
//! * completed: native equals `run_sb` (exit index, target, registers, flags, `insns`, memory,
//!   stale queue, auto-mapping);
//! * leaves at pc `P` after `R` retired instructions: native has `r15 = P`, `insns + R`, the
//!   registers, flags and memory tier 2 had just before the uop, and mapped nothing.
//!
//! The classification is the spec of the fast path written down independently of the lowerer:
//! each access is fast when its page is mapped and has no `HOOK_R` (load) or `HOOK_W`/`HOOK_C`
//! (store), and a uop with several accesses (`LDRD`, `STRD`, `LDM`, `STM`) needs them in one page.

use crate::cpu::{Bus, Cpu};
use crate::machine::sb::bus::SbBus;
use crate::machine::sb::exec_alu::exec_dp;
use crate::machine::sb::exec_flow::{exec_flow, Lap, Step};
use crate::machine::sb::exec_mem::exec_mem;
use crate::machine::sb::uop::fused::expand;
use crate::machine::sb::uop::{k, pack_flags, unpack_flags, Sb, Uop};
use crate::machine::SysBus;
use crate::mem::{Mem, HOOK_C, HOOK_R, HOOK_W};

/// Why a memory uop is not taken by the fast path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slow {
    Unmapped,
    HookRead,
    HookWrite,
    Code,
    PageCross,
}

/// What a differential run saw on the oracle: the "exercised" rule (a comparison that never
/// reached the interesting cases proves nothing).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    pub cases: u64,
    pub completed: u64,
    pub slow_exits: u64,
    pub slow: [u64; 5],
    pub loads: u64,
    pub stores: u64,
    pub unaligned_word_loads: u64,
    pub odd_halfwords: u64,
    pub blocks: u64,
    pub pairs: u64,
    pub writebacks: u64,
    pub guards_taken: u64,
    pub guards_passed: u64,
    pub preds_skipped: u64,
    pub preds_run: u64,
    pub back_edges: u64,
    pub exits_by_index: [u64; 8],
}

impl Tally {
    pub fn merge(&mut self, o: &Tally) {
        self.cases += o.cases;
        self.completed += o.completed;
        self.slow_exits += o.slow_exits;
        for i in 0..5 {
            self.slow[i] += o.slow[i];
        }
        self.loads += o.loads;
        self.stores += o.stores;
        self.unaligned_word_loads += o.unaligned_word_loads;
        self.odd_halfwords += o.odd_halfwords;
        self.blocks += o.blocks;
        self.pairs += o.pairs;
        self.writebacks += o.writebacks;
        self.guards_taken += o.guards_taken;
        self.guards_passed += o.guards_passed;
        self.preds_skipped += o.preds_skipped;
        self.preds_run += o.preds_run;
        self.back_edges += o.back_edges;
        for i in 0..8 {
            self.exits_by_index[i] += o.exits_by_index[i];
        }
    }

    /// Count of slow exits of one reason.
    pub fn slow_of(&self, why: Slow) -> u64 {
        self.slow[why as usize]
    }
}

/// What a probe load of a word returns: a word load rotated by an unaligned address changes it.
const PROBE_WORD: u32 = 0x0102_0304;

/// A bus that records the accesses of one uop and performs none.
#[derive(Default)]
struct Probe {
    log: Vec<(u32, bool, u32)>,
}

impl Bus for Probe {
    fn read8(&mut self, a: u32) -> u8 {
        self.log.push((a, false, 1));
        0
    }
    fn read16(&mut self, a: u32) -> u16 {
        self.log.push((a, false, 2));
        0
    }
    fn read32(&mut self, a: u32) -> u32 {
        self.log.push((a, false, 4));
        PROBE_WORD
    }
    fn write8(&mut self, a: u32, _: u8) {
        self.log.push((a, true, 1));
    }
    fn write16(&mut self, a: u32, _: u16) {
        self.log.push((a, true, 2));
    }
    fn write32(&mut self, a: u32, _: u32) {
        self.log.push((a, true, 4));
    }
    fn fetch16(&mut self, _: u32) -> u16 {
        0
    }
    fn fetch32(&mut self, _: u32) -> u32 {
        0
    }
}

impl SbBus for Probe {
    fn set_pc(&mut self, _: u32) {}
    fn stale(&self) -> bool {
        false
    }
}

/// `None` when the fast path takes memory uop `u` from state `r`, else the reason it does not
/// (the first one in access order). Also reports the accesses (for the tally).
fn classify(r: &[u32; 16], mem: &Mem, u: &Uop) -> (Option<Slow>, Vec<(u32, bool, u32)>) {
    let mut regs = *r;
    let mut probe = Probe::default();
    exec_mem(&mut regs, &mut probe, u);
    let log = probe.log;
    ROTATED.with(|c| {
        c.set(
            matches!(u.kind, k::LDR_IMM | k::LDR_REG | k::LDR_ABS)
                && regs[u.rd as usize] != PROBE_WORD,
        );
    });
    let first = log.first().map_or(0, |a| a.0 >> 12);
    for (a, store, size) in &log {
        let e = mem.entry(*a);
        if e & 0x0FFF_FFFF == 0 {
            return (Some(Slow::Unmapped), log);
        }
        if !*store && e & HOOK_R != 0 {
            return (Some(Slow::HookRead), log);
        }
        if *store && e & HOOK_W != 0 {
            return (Some(Slow::HookWrite), log);
        }
        if *store && e & HOOK_C != 0 {
            return (Some(Slow::Code), log);
        }
        let _ = size;
        if a >> 12 != first {
            return (Some(Slow::PageCross), log);
        }
    }
    (None, log)
}

thread_local! {
    /// Whether the word load classified last was rotated (an unaligned address).
    static ROTATED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The halfword address of a `LDRH`/`LDRSH`/`STRH` uop before the `& !1` (for the tally only).
fn half_address(r: &[u32; 16], u: &Uop) -> Option<u32> {
    if !matches!(u.kind, k::LDRH | k::LDRSH | k::STRH) {
        return None;
    }
    let off = if u.rs & 8 != 0 {
        r[u.rm as usize]
    } else {
        u.imm
    };
    let base = r[u.rn as usize];
    let moved = if u.rs & 1 != 0 {
        base.wrapping_add(off)
    } else {
        base.wrapping_sub(off)
    };
    Some(if u.rs & 2 != 0 { moved } else { base })
}

/// Where the oracle stopped.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Walk {
    /// The trace ran to an exit.
    Complete,
    /// Native code leaves for the interpreter at `pc` after `retired` instructions.
    Slow { pc: u32, retired: u64, why: Slow },
}

/// Tally one fast memory uop that the oracle executes.
fn note_access(t: &mut Tally, u: &Uop, regs: &[u32; 16], log: &[(u32, bool, u32)]) {
    for (_, store, _) in log {
        if *store {
            t.stores += 1;
        } else {
            t.loads += 1;
        }
    }
    t.unaligned_word_loads += u64::from(ROTATED.with(std::cell::Cell::get));
    t.odd_halfwords += u64::from(half_address(regs, u).is_some_and(|a| a & 1 != 0));
    t.blocks += u64::from(matches!(u.kind, k::LDM | k::STM));
    t.pairs += u64::from(matches!(u.kind, k::LDRD | k::STRD));
    t.writebacks += u64::from(u.rs & 4 != 0 && u.kind != k::LDR_ABS);
}

/// Run `sb` on tier 2 up to the first memory uop the fast path does not take (see the module
/// docs). `bus` and `cpu` are left as tier 2 had them just before that uop (flags written back),
/// or as `run_sb` would leave the registers when the trace completes (no `finish`: use
/// [`run_sb`] on a second copy for that).
pub(crate) fn walk(
    sb: &Sb,
    cpu: &mut Cpu,
    bus: &mut SysBus,
    left: u64,
    watch: bool,
    t: &mut Tally,
) -> Walk {
    let mut f = pack_flags(cpu);
    let mut lap = Lap::start(left, watch);
    let (mut i, mut retired) = (0usize, 0u64);
    loop {
        let u = &sb.uops[i];
        match k::family(u.kind) {
            k::Family::Dp => {
                exec_dp(&mut cpu.r, &mut f, u);
                retired += u64::from(u.n);
                i += 1;
            }
            k::Family::Mem => {
                if let Some(w) = step_mem(cpu, bus, &mut f, u, retired, t) {
                    return w;
                }
                retired += u64::from(u.n);
                i += 1;
            }
            k::Family::Fused if u.kind == k::LDR_PAIR => {
                for (j, part) in expand(u).expect("a fused kind").iter().enumerate() {
                    if let Some(w) = step_mem(cpu, bus, &mut f, part, retired + j as u64, t) {
                        return w;
                    }
                }
                retired += 2;
                i += 1;
            }
            k::Family::Flow | k::Family::Fused => {
                match exec_flow(cpu, bus, sb, i, &mut f, &mut lap) {
                    Step::Next(n) => {
                        if u.kind == k::PRED {
                            if n == i + 1 {
                                t.preds_run += 1;
                            } else {
                                t.preds_skipped += 1;
                                retired += sb.uops[i + 1..n]
                                    .iter()
                                    .map(|x| u64::from(x.n))
                                    .sum::<u64>();
                            }
                        }
                        if matches!(
                            u.kind,
                            k::GUARD
                                | k::GUARD_REG_EQ
                                | k::CMP_GUARD_I
                                | k::CMP_GUARD_R
                                | k::TST_GUARD_I
                        ) {
                            t.guards_passed += 1;
                        }
                        t.back_edges += u64::from(u.kind == k::LOOP_BACK);
                        retired += u64::from(u.n);
                        i = n;
                    }
                    Step::Exit(e) => {
                        t.guards_taken += u64::from(u.kind != k::LOOP_BACK && u.kind != k::EXIT);
                        t.exits_by_index[e.min(7)] += 1;
                        return Walk::Complete;
                    }
                }
            }
            k::Family::Invalid => unreachable!("kind {}", u.kind),
        }
    }
}

/// Execute memory uop `u` unless the fast path would not take it.
fn step_mem(
    cpu: &mut Cpu,
    bus: &mut SysBus,
    f: &mut u8,
    u: &Uop,
    retired: u64,
    t: &mut Tally,
) -> Option<Walk> {
    let (slow, log) = classify(&cpu.r, &bus.mem, u);
    if let Some(why) = slow {
        unpack_flags(cpu, *f);
        t.slow[why as usize] += 1;
        return Some(Walk::Slow {
            pc: u.pc,
            retired,
            why,
        });
    }
    note_access(t, u, &cpu.r, &log);
    exec_mem(&mut cpu.r, bus, u);
    None
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn merging_tallies_adds_every_counter_and_slow_of_indexes_by_reason() {
        let mut a = Tally::default();
        let mut b = Tally::default();
        (b.cases, b.loads, b.back_edges) = (3, 5, 2);
        b.slow[Slow::Code as usize] = 4;
        b.exits_by_index[7] = 1;
        a.merge(&b);
        a.merge(&b);
        assert_eq!((a.cases, a.loads, a.back_edges), (6, 10, 4));
        assert_eq!((a.slow_of(Slow::Code), a.slow_of(Slow::Unmapped)), (8, 0));
        assert_eq!(a.exits_by_index[7], 2);
    }
}
