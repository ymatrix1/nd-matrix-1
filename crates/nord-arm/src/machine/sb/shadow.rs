//! OWNER: W3-VERIF. Lockstep against the interpreter over real firmware, with bisection to one
//! instruction (skeleton: fail-closed, it reports that nothing was compared).
//!
//! Spec (design note 8, layer L3, WP W3-VERIF): `lockstep` runs the tiered machine in chunks of
//! `chunk` instructions; before each chunk it clones the `Machine`, runs the reference (T0:
//! `predecode = false`, tiers off) on the clone and compares the observable state (registers,
//! flags, `q`, `thumb`, `irq_off`, `fiq_off`, `mode`, `mode_changes`, `spsr`, `cp15_ctrl`,
//! `insns`, the fault, `Mem::state_hash`, `device_reads`, `poll_hits`, messages,
//! `auto_mapped(_count)`, `tick_t`) after the chunk. On a mismatch `bisect` restores the saved
//! state and halves the chunk until one instruction differs, and reports pc, word and the
//! register and flag deltas. A divergence must name the field that differs.

use super::config::SbConfig;
use super::SbState;
use crate::machine::Machine;

/// The first observable difference between the tiered run and the reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// `cpu.insns` at the end of the divergent instruction (or chunk).
    pub at_insn: u64,
    pub pc: u32,
    /// Readable description of the differing field(s).
    pub field: String,
}

fn unimplemented_divergence(what: &str) -> Divergence {
    Divergence {
        at_insn: 0,
        pc: 0,
        field: format!("{what}: not implemented yet (OWNER W3-VERIF), nothing was compared"),
    }
}

/// Run `total` instructions in lockstep chunks; `Ok(instructions compared)` when nothing
/// differed. The skeleton fails closed: it never claims a comparison it did not make.
pub fn lockstep(
    m: &mut Machine,
    sb: &mut SbState,
    chunk: u64,
    total: u64,
    until: u32,
    stop: Option<u32>,
) -> Result<u64, Divergence> {
    let _ = (m, sb, chunk, total, until, stop);
    Err(unimplemented_divergence("lockstep"))
}

/// Narrow a divergent chunk that starts at `before` to the first divergent instruction.
pub fn bisect(
    before: &Machine,
    sb_cfg: &SbConfig,
    chunk: u64,
    until: u32,
    stop: Option<u32>,
) -> Divergence {
    let _ = (before, sb_cfg, chunk, until, stop);
    unimplemented_divergence("bisect")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skeleton_fails_closed_and_names_its_owner() {
        let mut m = Machine::new();
        let mut sb = SbState::default();
        let e = lockstep(&mut m, &mut sb, 100, 1000, 0, None).unwrap_err();
        assert!(e.field.contains("W3-VERIF") && e.field.contains("nothing was compared"));
        let d = bisect(&m, &SbConfig::default(), 100, 0, None);
        assert!(d.field.starts_with("bisect"));
    }
}
