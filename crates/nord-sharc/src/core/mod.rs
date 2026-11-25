//! Execution core of the SHARC oracle (milestone M2 of `docs/specs/sharc-core.md`).
//!
//! Runs VISA code straight from the raw parcels of a flat SRAM image: no decoded-instruction
//! objects, no strings and no allocation in [`Core::step`], so it is fast enough to run whole DSP
//! start-up sequences and single routines in tests. Anything not implemented yet stops the run
//! with a [`Fault`] naming the program counter and the raw parcels, never with wrong results.
//!
//! Provenance of the modelled semantics: instruction layouts are the ones verified in
//! `visa.rs`; behaviour follows the SHARC Processor Programming Reference rev 2.4. Float results
//! use the 40-bit extended format with the MODE1 TRUNC / RND32 rounding modes (`float.rs`).
//!
//! Layout: this file holds the state and the small shared helpers; `memory` the data memory and
//! register files, `dag` address generation, `alu` and `multiplier` the compute units, `moves`
//! the data and register transfers, `branch` jumps, calls, loops and delay slots, and `exec` the
//! single-step decoder that dispatches to them. The sequencer system (`interrupts` controller and
//! IVT dispatch, `timer` and cycle clock, `sysregs` system registers and stacks, `banks` secondary
//! register sets) is described in `docs/research/sharc-sequencer.md`.

mod addr_hash;
mod alu;
mod aot;
mod aot_emit;
mod aot_fast;
mod aot_glue;
pub(crate) mod aot_hot;
mod aot_lower;
pub(crate) mod aot_mem;
mod aot_native;
mod aot_pe;
pub mod aot_prof;
mod aot_state;
mod aot_whole;
mod arith_loop;
mod banks;
mod bitfifo;
mod branch;
mod branch_ext;
pub mod census;
#[cfg(test)]
mod census_engines;
#[cfg(test)]
mod census_regress;
#[cfg(test)]
mod compute_adi;
mod condition;
mod dag;
mod dag_modes;
mod decode_cache;
mod dualmove;
mod exec;
#[cfg(test)]
mod fixed2_measured;
mod fixed_alu;
#[cfg(test)]
mod fixed_measured;
mod float;
mod float_conv;
#[cfg(test)]
mod float_flags;
#[cfg(test)]
mod float_measured;
mod float_misc;
mod float_mul;
mod float_ops;
mod float_seeds;
mod interrupts;
#[cfg(test)]
mod irq_scenarios;
mod isa;
#[cfg(feature = "jit")]
mod jit;
mod reg_moves;
#[cfg(feature = "jit")]
pub use jit::{JitProfile, PassTraceReport};
#[cfg(all(test, feature = "jit"))]
mod jit_diff;
mod longword;
#[cfg(test)]
mod loops_measured;
mod memory;
mod moves;
mod moves_types;
mod multifunction;
mod multiplier;
mod new;
mod pcstack;
pub mod periph;
pub mod probe_fixture;
pub mod sdram;
mod shifter;
mod shifter_bits;
#[cfg(test)]
mod simd_measured;
mod site_hits;
#[cfg(test)]
mod stack_measured;
mod stacks;
#[cfg(test)]
pub(crate) mod support;
mod sysregs;
mod timer;
mod types;
mod vectors;

use crate::visa::Insn;
use addr_hash::DeviceMap;
pub use aot::AotOptions;
pub use aot_hot::AotHotStats;
pub use aot_lower::{AotTier, InsnClass};
use types::sign_extend;
pub use types::{Fault, Flags};
use types::{LoopFrame, Pending};
pub use vectors::{vector_by_name, VECTOR_COUNT};

/// Bytes of the flat SRAM image: index = `address & 0x7FFFFF` (loader address), byte `2 * short`.
pub const SRAM_BYTES: usize = 0x40_0000;
/// 32-bit words of the IOP register window (`0x0000_0000..0x0003_FFFF` normal-word addresses).
const IOP_WORDS: usize = 0x4_0000;
/// Most skipped operations whose pc is logged in lenient mode.
pub const SKIP_LOG_LIMIT: usize = 100_000;
/// MODE1 bit 21: processing element Y enabled (SIMD mode).
const MODE1_PEYEN: u32 = 1 << 21;
/// Most writes kept by the data watch.
pub const WATCH_LIMIT: usize = 64;

/// Architectural state of one SHARC core (processing element X; PEy is not modelled yet).
pub struct Core {
    /// Program counter, a short-word address.
    pub pc: u32,
    pub r: [u32; 16],
    /// PEy data registers (S0-S15), reached only through the `Dreg <-> CDreg` swap for now.
    pub s: [u32; 16],
    pub i: [u32; 16],
    pub m: [u32; 16],
    pub l: [u32; 16],
    pub b: [u32; 16],
    pub mode1: u32,
    pub mode2: u32,
    pub ustat: [u32; 4],
    /// Multiplier result registers MRF (index 0) and MRB (index 1), 80 bits sign-extended.
    pub mr: [i128; 2],
    /// When set, compute operations that are not modelled are skipped and counted instead of faulting.
    pub lenient: bool,
    /// When set, the Level-2 JIT lowers `FloatMul` and `FADD`/`FSUB` to native IEEE-754 `f32`
    /// instead of calling 40-bit software helpers.
    pub fast_float: bool,
    /// SHARC+ core (ADSP-2157x): the system-register write masks differ from the ADSP-2148x
    /// (`sysregs.rs`). Set by `enable_sc57x` and by a `plus` line in census cases.
    pub sharc_plus: bool,
    /// Number of skipped unmodelled compute operations and the pc of the first one.
    pub skipped: u64,
    pub first_skipped: Option<u32>,
    /// Program counters of skipped operations (first [`SKIP_LOG_LIMIT`]), for histograms.
    pub skipped_pcs: Vec<u32>,
    /// Data address whose writes are logged in `watch_hits` (`u32::MAX` = none).
    pub watch: u32,
    /// Number of consecutive words watched from `watch` (1 = just that address).
    pub watch_len: u32,
    /// Writes to the watched address as `(step, pc, value)`, at most [`WATCH_LIMIT`].
    pub watch_hits: Vec<(u64, u32, u32)>,
    /// Address of each entry of `watch_hits` (differs from `watch` when `watch_len > 1`).
    pub watch_addrs: Vec<u32>,
    /// Maximum number of logged writes (default [`WATCH_LIMIT`]).
    pub watch_limit: usize,
    /// First data address whose loads are logged in `rwatch_hits` while `rwatch_on` (`u32::MAX` = none).
    pub rwatch: u32,
    /// Register trace window `(lo, hi, limit)`: every step whose pc lies in `lo..=hi` logs R, S, I, M.
    pub trace: Option<(u32, u32, usize)>,
    /// Data words `(address, count)` appended to every `trace` line (`count` 0 = none).
    pub trace_dm: (u32, u32),
    /// Lines logged by `trace`.
    pub trace_log: Vec<String>,
    /// Number of consecutive words load-watched from `rwatch`.
    pub rwatch_len: u32,
    /// Loads inside the window as `(pc, address)`, at most `watch_limit`.
    pub rwatch_hits: Vec<(u32, u32)>,
    /// True while the pass loop runs (host reads of the memory are not core loads).
    pub rwatch_on: bool,
    pub imask: u32,
    pub mmask: u32,
    pub flags: Flags,
    /// PEy condition flags and multiplier result registers (used in SIMD mode).
    pub flags_y: Flags,
    pub mr_y: [i128; 2],
    /// Low 8 bits of the 40-bit float registers of PEx and PEy (see `float_ops`).
    pub(super) fx: float_ops::ExtFile,
    fx_y: float_ops::ExtFile,
    /// Number of instructions executed.
    pub steps: u64,
    /// Opt-in per-pc execution counters ([`site_hits`]); empty = off.
    site_hits: Vec<(u32, u64)>,
    sram: Box<[u8]>,
    iop: Box<[u32]>,
    /// Sparse store for addresses outside the SRAM and IOP windows (peripherals, external bus).
    device: DeviceMap,
    /// Dense external delay-line window `0x0800_0000..` ([`sdram`]).
    sdram: sdram::Sdram,
    /// Reads and writes that went to the SDRAM window (diagnostic counters).
    pub sdram_reads: u64,
    pub sdram_writes: u64,
    /// Values returned by device reads regardless of earlier writes (scripted status registers).
    preset: DeviceMap,
    /// Reads and writes that went to the device store (diagnostic counters).
    pub device_reads: u64,
    pub device_writes: u64,
    call_stack: Vec<u32>,
    pub(super) loops: Vec<LoopFrame>,
    pending: Option<Pending>,
    irq: interrupts::IrqState,
    timer: timer::TimerState,
    pub(in crate::core) sys: sysregs::SysState,
    banks: banks::Banks,
    /// Circular buffer wraps of `I7` (bit 0) and `I15` (bit 1) since the last instruction ended
    /// (set while an address is computed, see `dag_modes`).
    cb_wrap: std::cell::Cell<u8>,
    /// ISA-mode program memory ([`isa`]): 3 parcels (most significant first) per 48-bit word.
    /// Empty = VISA mode, where `pc` addresses the flat SRAM image in parcels.
    isa_code: Vec<u16>,
    /// Optional peripheral models ([`periph`]); `None` keeps the plain device store.
    periph: Option<Box<periph::Periph>>,
    /// Direct-mapped Level-1 decode cache (allocated lazily on first `step`).
    decode_cache: Option<Box<decode_cache::DecodeCache>>,
    /// Monotonic generation for O(1) cache invalidation on ISA/code reloads.
    code_gen: u64,
    /// Bounding normal-word range `[lo, hi]` of all SRAM instructions cached in `decode_cache`
    /// or compiled into `jit` (`lo > hi` when empty). Inline SRAM stores inside this range fall
    /// back to `dm_write` so `invalidate_sram_cache` stays exact across all cached blocks.
    sram_code_nw_lo: u32,
    sram_code_nw_hi: u32,
    /// Optional hot-block AOT hook ([`aot_hot`]); `None` keeps the plain interpreter.
    aot_hot: Option<Box<aot_hot::AotHot>>,
    /// Entry bitmap of the enabled blocks: bit `pc - aot_lo` is set at each block start;
    /// `aot_span` is 0 while none is enabled, so the per-step test is one compare.
    aot_lo: u32,
    aot_span: u32,
    aot_bits: Box<[u64]>,
    /// Set when a lowered AOT store left the internal SRAM (`aot_mem.rs`); the generated block
    /// then re-checks its guard signature.
    aot_slow: bool,
    /// Optional Level-2 Cranelift JIT engine (allocated lazily on first `run_jit`).
    #[cfg(feature = "jit")]
    jit: Option<Box<jit::JitEngine>>,
}

impl Core {
    /// Instruction parcel at a short-word address (for tools that decode skipped operations).
    pub fn parcel_at(&self, short: u32) -> u16 {
        self.parcel(short)
    }

    #[inline]
    fn parcel(&self, short: u32) -> u16 {
        let o = (short as usize) * 2;
        u16::from_le_bytes([
            self.sram[o & (SRAM_BYTES - 1)],
            self.sram[(o + 1) & (SRAM_BYTES - 1)],
        ])
    }

    /// Whether SIMD mode (PEy enabled) is on.
    #[inline]
    fn simd(&self) -> bool {
        self.mode1 & MODE1_PEYEN != 0
    }

    /// Exchanges the PEx and PEy views (data registers, flags, multiplier results), so the code
    /// written for PEx can run unchanged on PEy and the views are swapped back afterwards.
    fn swap_pe(&mut self) {
        std::mem::swap(&mut self.r, &mut self.s);
        std::mem::swap(&mut self.flags, &mut self.flags_y);
        std::mem::swap(&mut self.mr, &mut self.mr_y);
        std::mem::swap(&mut self.fx, &mut self.fx_y);
        // MN/MV/MU/MI, AF and the sticky bits belong to the element too
        self.sys.astat_extra.swap(0, 1);
        self.sys.stky.swap(0, 1);
    }

    fn fault(&self, pc: u32, insn: &Insn) -> Fault {
        Fault::Unimplemented {
            pc,
            parcels: insn.parcels,
        }
    }

    /// Skips an unmodelled compute operation in lenient mode, otherwise reports it as a fault.
    fn unmodelled(&mut self, pc: u32, insn: &Insn) -> Result<(), Fault> {
        if !self.lenient {
            return Err(self.fault(pc, insn));
        }
        self.skipped += 1;
        self.first_skipped.get_or_insert(pc);
        if self.skipped_pcs.len() < SKIP_LOG_LIMIT {
            self.skipped_pcs.push(pc);
        }
        Ok(())
    }

    /// Runs until a fault or `max_steps` instructions; returns the fault, if any.
    pub fn run(&mut self, max_steps: u64) -> Option<Fault> {
        let limit = self.steps + max_steps;
        while self.steps < limit {
            if let Err(f) = self.step() {
                return Some(f);
            }
        }
        None
    }

    /// Runs using the Level-2 Cranelift JIT when the `jit` feature is enabled, otherwise falls
    /// back to [`Core::run`].
    pub fn run_auto(&mut self, max_steps: u64) -> Option<Fault> {
        #[cfg(feature = "jit")]
        {
            self.run_jit(max_steps)
        }
        #[cfg(not(feature = "jit"))]
        {
            self.run(max_steps)
        }
    }

    /// Advances the core by `cycles` cycles using the JIT when `use_jit` and the `jit` feature
    /// are enabled, or the interpreter otherwise.
    pub fn run_for_cycles(&mut self, cycles: u64, use_jit: bool) -> Option<Fault> {
        if use_jit {
            self.run_auto(cycles)
        } else {
            self.run(cycles)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::support::{core_at_zero, core_with};
    use super::*;

    #[test]
    fn sign_extension_and_conditions() {
        assert_eq!(sign_extend(0xFFFF, 16), 0xFFFF_FFFF);
        assert_eq!(sign_extend(0x7FFF, 16), 0x7FFF);
        let mut c = core_with(&[0x0001]);
        assert!(c.cond(31) && !c.cond(0) && c.cond(16));
        c.flags.az = true;
        assert!(c.cond(0) && !c.cond(16) && c.cond(2));
        c.flags.az = false;
        c.flags.an = true;
        assert!(c.cond(1) && c.cond(2) && !c.cond(17));
    }

    #[test]
    fn unimplemented_instruction_is_reported_not_guessed() {
        let mut c = core_with(&[0xd70a, 0x3c23, 0xd70a]);
        assert!(matches!(
            c.step(),
            Err(Fault::Unimplemented { pc: 0x12_0000, .. })
        ));
        assert_eq!(
            c.pc, 0x12_0000,
            "a faulting instruction does not advance the pc"
        );
    }

    #[test]
    fn lenient_mode_skips_unmodelled_compute_and_counts_it() {
        // type 2a word `IF TRUE` with compute 0x40_3000: an MR transfer of MR3, which does not exist
        let parcels = [0x013eu16, 0x0040, 0x3000, 0x0000];
        let mut strict = core_at_zero(&parcels);
        assert!(strict.step().is_err());
        let mut c = core_at_zero(&parcels);
        c.lenient = true;
        assert!(c.step().is_ok());
        assert_eq!((c.skipped, c.first_skipped), (1, Some(0)));
    }

    #[test]
    fn run_stops_at_the_step_limit() {
        let mut c = core_with(&[0x0001, 0x0001, 0x0001]);
        assert_eq!(c.run(2), None);
        assert_eq!(c.steps, 2);
    }
}
