//! OWNER: W2-XCORE. Executor arms of the control uops and of the fused uops, tier 2 (safe Rust).
//!
//! Spec (design note 4.3, WP W2-XCORE): [`exec_flow`] runs `sb.uops[i]` of a `Flow` or `Fused`
//! kind (`k::family`) with the flag nibble in `f` (I4) and says where the executor goes next
//! ([`Step`]):
//!
//! | Kind | Arm |
//! |---|---|
//! | `PRED` | the next `imm` uops run when `holds(cond, f)`, else they are skipped (the instruction still retires: its `done` is static) |
//! | `GUARD` | leave through exit `rs` when `holds(cond, f)` |
//! | `GUARD_REG_EQ` | leave through exit `rs` when `r[rn] != imm` (an inlined return that does not go where the trace assumed) |
//! | `LOOP_BACK` | re-enter at uop 0 while `left >= acc + n_guest` ([`Lap`]), else leave through exit `rs`; with `Lap::watch` it always leaves (idle v2) |
//! | `EXIT`, `EXIT_REG`, `EXIT_REG_ARM` | leave through exit `rs`; the target is computed by `exec::exit_target` from the exit kind (`Indirect` switches to Thumb on bit 0) |
//! | `GENERIC` | a non-ender through `exec_pure` with the nibble spilled and reloaded; leaves through its early exit (`cond`) when a store made code stale (I6) |
//! | `GENERIC_END` | an ender through `exec_decoded` with the nibble spilled and reloaded; leaves through its own `Dynamic` exit (`rs`); a fault stays in `cpu.fault` |
//! | `MRS`, `MSR_C` | `psr.rs` |
//! | `CMP_GUARD_*`, `TST_GUARD_I`, `MOV_PAIR`, `LDR_PAIR` | `fused.rs`: each is its unfused expansion (`uop::fused::expand`) |
//!
//! An unknown kind is `unreachable!` (the registry `k::family` keeps it out; a test enumerates
//! all 256 ids). No arm reads or writes `r15` (I2): `GENERIC*` leave `r[15] = pc + 8` or the jump
//! target behind, which the exit overwrites.
//!
//! # `Lap`: the loop accumulator
//!
//! `Exit::done` is the instructions of the current iteration (I3), so after `acc` instructions of
//! completed iterations every exit retires `acc + done`. `LOOP_BACK` first adds its own
//! iteration (`done` of its exit) and then enters another one only when the budget still covers
//! a full worst-case iteration after it (`left >= acc + n_guest`, I7 for the next lap); the exit
//! it takes is applied by `exec::finish` with the same `acc`. `Lap::left` is `*left` at entry: the
//! executor decrements it once, at the exit.
//!
//! # `MRS` and `MSR_C` layout (design note 3.1: "Rd | Rm | | b0 | | rotated immediate")
//!
//! | Kind | `rd` | `rn` | `rs` bit 0 | `imm` |
//! |---|---|---|---|---|
//! | `MRS` | destination | | 0: CPSR, 1: SPSR | |
//! | `MSR_C` (`msr cpsr_c`) | | Rm (register form) | 0: register form, 1: immediate form | the rotated immediate (immediate form) |
//!
//! # Mutation sites (L5, ids `400..=449` of `mutants.rs`)
//!
//! [`MUTANTS`] lists the sites of this file and its children (`exec.rs` has `450..=499`);
//! `mutant(id)` is false outside a test build. `exec/kills.rs` proves each one is killed.
//!
//! Registered statically in `exec::run_sb` (families `Flow` and `Fused`); `Step`, `Lap` and the
//! signature belong to this work package and `exec.rs`.

use super::bus::SbBus;
use super::uop::{holds, k, pack_flags, unpack_flags, Sb};
use crate::cpu::predecode::decode_arm;
use crate::cpu::Cpu;

#[cfg(test)]
pub(super) mod cases;
pub(super) mod fused;
pub(super) mod psr;

/// What the executor loop does after one control uop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Continue at this uop index.
    Next(usize),
    /// Leave through `sb.exits[exit]`.
    Exit(usize),
}

/// The loop state `LOOP_BACK` works on (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lap {
    /// Guest instructions of the iterations completed so far.
    pub acc: u64,
    /// `*left` when the trace was entered.
    pub left: u64,
    /// Idle v2: `LOOP_BACK` returns to the driver at the loop head instead of re-entering.
    pub watch: bool,
}

impl Lap {
    /// The state at the start of a run: no completed iteration.
    pub const fn start(left: u64, watch: bool) -> Self {
        Self {
            acc: 0,
            left,
            watch,
        }
    }
}

/// `(id, name)` of the mutation sites of `exec_flow.rs` and its children (block `400..=449`).
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (400, "pred: skips one uop too many"),
    (401, "pred: condition inverted"),
    (402, "guard: condition inverted"),
    (403, "guard_reg_eq: exits when equal"),
    (404, "loop_back: budget test ignores n_guest"),
    (405, "loop_back: iterations are not accumulated"),
    (406, "generic: no early exit after a store"),
    (407, "generic: the nibble is not reloaded"),
    (408, "generic_end: the nibble is not spilled"),
    (409, "mrs: reads the flags of the cpu, not the nibble"),
    (410, "mrs: the SPSR bit is ignored"),
    (411, "msr_c: writes the T bit"),
    (412, "msr_c: mode changes are not counted"),
    (413, "msr_c: the immediate form reads the register"),
    (414, "msr_c: writes the flag bits"),
    (415, "cmp_guard: condition inverted"),
    (416, "cmp_guard: the compare does not write the flags"),
    (417, "mov_pair: halves swapped"),
    (418, "ldr_pair: second load at the pc of the first"),
    (419, "ldr_pair: second load into the first destination"),
    (420, "tst_guard: keeps the old carry"),
    (421, "loop_back: ignores the watch request"),
];

/// Mutation site: true only inside `mutants::with(id, ..)` of a test build.
#[cfg(test)]
#[inline(always)]
pub(super) fn mutant(id: u32) -> bool {
    super::mutants::is_active(id)
}

/// Mutation site: false outside a test build, so the wrong branch is dead code there.
#[cfg(not(test))]
#[inline(always)]
pub(super) const fn mutant(_id: u32) -> bool {
    false
}

/// Execute `sb.uops[i]` (a `Flow` or `Fused` kind) with the flag nibble in `f`.
#[inline]
pub fn exec_flow<B: SbBus>(
    cpu: &mut Cpu,
    bus: &mut B,
    sb: &Sb,
    i: usize,
    f: &mut u8,
    lap: &mut Lap,
) -> Step {
    let u = &sb.uops[i];
    match u.kind {
        k::PRED => {
            if holds(u.cond, *f) != mutant(401) {
                Step::Next(i + 1)
            } else {
                Step::Next(i + 1 + u.imm as usize + usize::from(mutant(400)))
            }
        }
        k::GUARD => {
            if holds(u.cond, *f) != mutant(402) {
                Step::Exit(u.exit_idx())
            } else {
                Step::Next(i + 1)
            }
        }
        k::GUARD_REG_EQ => {
            if (cpu.r[u.rn as usize] != u.imm) != mutant(403) {
                Step::Exit(u.exit_idx())
            } else {
                Step::Next(i + 1)
            }
        }
        k::LOOP_BACK => {
            let idx = u.exit_idx();
            let acc = lap.acc + u64::from(sb.exits[idx].done);
            let room = if mutant(404) {
                lap.left >= acc
            } else {
                lap.left >= acc + u64::from(sb.n_guest)
            };
            if room && (!lap.watch || mutant(421)) {
                if !mutant(405) {
                    lap.acc = acc;
                }
                Step::Next(0)
            } else {
                Step::Exit(idx)
            }
        }
        k::EXIT | k::EXIT_REG | k::EXIT_REG_ARM => Step::Exit(u.exit_idx()),
        k::GENERIC => {
            bus.set_pc(u.pc);
            unpack_flags(cpu, *f);
            cpu.exec_pure(bus, decode_arm(u.imm), u.pc);
            if !mutant(407) {
                *f = pack_flags(cpu);
            }
            if bus.stale() && !mutant(406) {
                Step::Exit(u.exit_idx())
            } else {
                Step::Next(i + 1)
            }
        }
        k::GENERIC_END => {
            bus.set_pc(u.pc);
            if !mutant(408) {
                unpack_flags(cpu, *f);
            }
            // a fault stays in `cpu.fault`; the driver takes it after the run
            cpu.exec_decoded(bus, decode_arm(u.imm), u.pc);
            *f = pack_flags(cpu);
            Step::Exit(u.exit_idx())
        }
        k::MRS => {
            psr::mrs(cpu, u, *f);
            Step::Next(i + 1)
        }
        k::MSR_C => {
            psr::msr_c(cpu, u);
            Step::Next(i + 1)
        }
        k::CMP_GUARD_I | k::CMP_GUARD_R | k::TST_GUARD_I => {
            if fused::cmp_guard(&mut cpu.r, f, u) {
                Step::Exit(u.exit_idx())
            } else {
                Step::Next(i + 1)
            }
        }
        k::MOV_PAIR => {
            fused::mov_pair(&mut cpu.r, u);
            Step::Next(i + 1)
        }
        k::LDR_PAIR => {
            fused::ldr_pair(&mut cpu.r, bus, u);
            Step::Next(i + 1)
        }
        kind => unreachable!("uop kind {kind} is not a control or fused uop (OWNER W2-XCORE)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lap_starts_empty_with_the_entry_budget() {
        assert_eq!(
            Lap::start(9, true),
            Lap {
                acc: 0,
                left: 9,
                watch: true
            }
        );
    }

    #[test]
    fn the_mutation_sites_are_off_outside_a_mutant_run_and_listed_once() {
        assert!(MUTANTS.iter().all(|(id, _)| !mutant(*id)));
        let mut ids: Vec<u32> = MUTANTS.iter().map(|m| m.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MUTANTS.len());
        assert!(ids.iter().all(|id| (400..=449).contains(id)));
    }
}
