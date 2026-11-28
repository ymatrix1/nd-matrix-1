//! Profile of a whole-pass AOT run (bean `ns4-lle3`): which instruction classes still run through
//! the interpreter, why, and what share of the pass time they cost. Off by default (one relaxed
//! atomic load on the fallback paths only); `lle_sharc_bench --profile` turns it on for one core.
//!
//! Causes: `generic` = an instruction inside a block that has no native lowering (`aot_step`),
//! `refused` = the pc is a block start but the entry guard or step limit refused it,
//! `unblocked` = the pc is in no block. Cycles are `rdtsc` counts, reported as shares.
use super::decode_cache::{DecodedOp, MoveOp};
use super::Core;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub const GENERIC: u8 = 0;
pub const REFUSED: u8 = 1;
pub const UNBLOCKED: u8 = 2;
/// The pc lies inside the range of an enabled block but is not its start (the rest of a block
/// after a bail-out).
pub const MID_BLOCK: u8 = 3;

static ON: AtomicBool = AtomicBool::new(false);
static COMPUTE_CYCLES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static COMPUTE_OPS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static DATA: Mutex<Prof> = Mutex::new(Prof::new());

struct Prof {
    /// `(pc, cause)` to `(steps, cycles)`.
    rows: BTreeMap<(u32, u8), (u64, u64)>,
    /// Entry-guard refusals by first failing condition.
    refusals: BTreeMap<&'static str, u64>,
    run_cycles: u64,
    try_cycles: u64,
    pass_cycles: u64,
    passes: u64,
}

impl Prof {
    const fn new() -> Self {
        Prof {
            rows: BTreeMap::new(),
            refusals: BTreeMap::new(),
            run_cycles: 0,
            try_cycles: 0,
            pass_cycles: 0,
            passes: 0,
        }
    }
}

/// Turns the profile on or off and clears it.
pub fn aot_prof_enable(on: bool) {
    *DATA.lock().unwrap() = Prof::new();
    for k in 0..2 {
        COMPUTE_CYCLES[k].store(0, Ordering::Relaxed);
        COMPUTE_OPS[k].store(0, Ordering::Relaxed);
    }
    ON.store(on, Ordering::Relaxed);
}

/// True while the profile is on.
#[inline(always)]
pub fn aot_prof_on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// The time stamp counter (0 on targets without one).
#[inline(always)]
pub fn tsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: `rdtsc` reads a counter and has no memory effects.
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

/// Records one fallback step at `pc`.
pub fn aot_prof_step(pc: u32, cause: u8, cycles: u64) {
    let mut d = DATA.lock().unwrap();
    let e = d.rows.entry((pc, cause)).or_default();
    e.0 += 1;
    e.1 += cycles;
}

/// Records one pass: its total cycles and the cycles of the block-entry attempts that ran a block.
pub fn aot_prof_pass(pass: u64, try_cycles: u64) {
    let mut d = DATA.lock().unwrap();
    d.pass_cycles += pass;
    d.try_cycles += try_cycles;
    d.passes += 1;
}

/// Records the cycles of one compute field run inside a lowered transfer.
pub(super) fn prof_compute(fast: bool, cycles: u64) {
    COMPUTE_CYCLES[usize::from(fast)].fetch_add(cycles, Ordering::Relaxed);
    COMPUTE_OPS[usize::from(fast)].fetch_add(1, Ordering::Relaxed);
}

pub(super) fn prof_run(cycles: u64) {
    DATA.lock().unwrap().run_cycles += cycles;
}

pub(super) fn prof_refusal(why: &'static str) {
    *DATA.lock().unwrap().refusals.entry(why).or_default() += 1;
}

/// One row of the report: instruction class, cause, steps and cycles.
#[derive(Debug, Clone)]
pub struct ProfRow {
    pub class: String,
    pub cause: &'static str,
    pub steps: u64,
    pub cycles: u64,
}

/// The profile of the run so far.
#[derive(Debug, Clone, Default)]
pub struct ProfReport {
    pub rows: Vec<ProfRow>,
    pub refusals: Vec<(&'static str, u64)>,
    pub run_cycles: u64,
    pub try_cycles: u64,
    pub pass_cycles: u64,
    pub passes: u64,
    /// Cycles and count of compute fields run inside lowered transfers (`aot_compute_both`).
    /// Index 0: interpreter `exec_compute_op`, 1: generated `Pe` kernel.
    pub compute_cycles: [u64; 2],
    pub compute_ops: [u64; 2],
}

/// First word of a `Debug` text: the variant name.
fn variant(s: &str) -> &str {
    s.split([' ', '(', '{']).next().unwrap_or("")
}

/// Name of an instruction class: `Flow:JumpDirect`, `Move:DagMem`, `Move:Other(4a)`, ...
fn class_name(op: &DecodedOp, kind: crate::visa::Kind) -> String {
    match op {
        DecodedOp::Flow(f) => format!("Flow:{}", variant(&format!("{f:?}"))),
        DecodedOp::Move(MoveOp::Other) => format!("Move:Other({})", kind.name()),
        DecodedOp::Move(m) => format!("Move:{}", variant(&format!("{m:?}"))),
        DecodedOp::ComputeCond { cond, .. } if *cond != 31 => "ComputeCond(conditional)".into(),
        other => variant(&format!("{other:?}")).to_string(),
    }
}

impl Core {
    /// True when the pc is the start of an enabled AOT block (the guard may still refuse it).
    pub fn aot_at_block_start(&self) -> bool {
        let o = self.pc.wrapping_sub(self.aot_lo);
        o < self.aot_span && self.aot_bits[(o >> 6) as usize] >> (o & 63) & 1 != 0
    }

    /// True when the pc lies inside the pc range of an enabled block.
    pub fn aot_pc_in_block(&self) -> bool {
        self.aot_hot.as_deref().is_some_and(|h| {
            let i = h.blocks.partition_point(|b| b.start <= self.pc);
            i > 0 && self.pc <= h.blocks[i - 1].last
        })
    }

    /// The profile gathered since [`aot_prof_enable`], classes named from this core's code.
    pub fn aot_prof_report(&self) -> ProfReport {
        let d = DATA.lock().unwrap();
        let mut by: BTreeMap<(String, u8), (u64, u64)> = BTreeMap::new();
        for (&(pc, cause), &(s, c)) in &d.rows {
            let (_, op, kind, ..) = self.whole_decode(pc);
            let e = by.entry((class_name(&op, kind), cause)).or_default();
            e.0 += s;
            e.1 += c;
        }
        let names = ["generic", "refused", "unblocked", "mid-block"];
        ProfReport {
            rows: by
                .into_iter()
                .map(|((class, cause), (steps, cycles))| ProfRow {
                    class,
                    cause: names[usize::from(cause)],
                    steps,
                    cycles,
                })
                .collect(),
            refusals: d.refusals.iter().map(|(&k, &v)| (k, v)).collect(),
            run_cycles: d.run_cycles,
            try_cycles: d.try_cycles,
            pass_cycles: d.pass_cycles,
            passes: d.passes,
            compute_cycles: [0, 1].map(|k| COMPUTE_CYCLES[k].load(Ordering::Relaxed)),
            compute_ops: [0, 1].map(|k| COMPUTE_OPS[k].load(Ordering::Relaxed)),
        }
    }

    /// Why the entry guard refuses the block at the pc (first failing condition), for the profile.
    pub(super) fn aot_refusal_reason(
        &self,
        b: &super::aot_hot::AotBlock,
        limit: u64,
    ) -> &'static str {
        let n = u64::from(b.steps);
        if self.steps + n > limit {
            "step limit"
        } else if self.pending.is_some() {
            "delayed branch pending"
        } else if self.irq.window_len != 0 || self.irq.idle {
            "interrupt window / idle"
        } else if !b.ev && self.steps + n > self.irq.due {
            "interrupt controller event due inside block"
        } else if self
            .loops
            .last()
            .is_some_and(|f| f.term.is_some() || super::Core::loop_end_blocks(b, f.end))
        {
            "hardware loop ends inside block"
        } else if self.arith_irq_enabled() {
            "arithmetic exception interrupt enabled"
        } else if !self.aot_code_matches(b) {
            "SRAM code changed"
        } else {
            "peripheral / watch / trace guard"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    #[test]
    fn rows_are_named_by_instruction_class_and_summed() {
        aot_prof_enable(true);
        let c = core_with(&[0x0001, 0x073e, 0x00ff, 0xfffa]);
        aot_prof_step(c.pc, GENERIC, 10);
        aot_prof_step(c.pc, GENERIC, 5);
        aot_prof_step(c.pc + 1, UNBLOCKED, 7);
        aot_prof_pass(100, 40);
        let r = c.aot_prof_report();
        aot_prof_enable(false);
        let nop = r.rows.iter().find(|x| x.class == "Nop").unwrap();
        assert_eq!((nop.steps, nop.cycles, nop.cause), (2, 15, "generic"));
        let jump = r
            .rows
            .iter()
            .find(|x| x.class == "Flow:JumpDirect")
            .unwrap();
        assert_eq!((jump.steps, jump.cause), (1, "unblocked"));
        assert_eq!((r.pass_cycles, r.try_cycles, r.passes), (100, 40, 1));
    }

    #[test]
    fn block_start_is_false_without_blocks() {
        assert!(!core_with(&[0x0001]).aot_at_block_start());
    }
}
