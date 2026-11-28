//! OWNER: W3-VERIF. Translation validation of a freshly built trace (skeleton: fail-closed).
//!
//! Spec (design note 8, layer L4, WP W3-VERIF): with `SbConfig::validate_cases = K` (default 0;
//! the real-firmware CI job uses 8) every newly built trace runs against the interpreter on `K`
//! random states over a private mini machine that holds copies of the trace's pages: registers
//! biased to 0, 1, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF and to addresses into mapped RAM, device
//! pages and unmapped pages, all 16 flag nibbles. The state after the trace (registers, flags,
//! `insns`, memory hash, access log) must equal the state after `Cpu::step` run for the same
//! number of instructions. It catches lowering bugs on real firmware shapes; a trace that fails
//! is dropped, its entry pc is marked failed and `SbStats::shadow_mismatches` is bumped.

use super::uop::Sb;
use crate::machine::Machine;

/// Validate `sb` on `cases` random states derived from `seed`. The skeleton returns an error
/// (a trace that was not validated is rejected, so `validate_cases > 0` stays safe before the
/// work package lands).
pub fn validate_trace(m: &Machine, sb: &Sb, cases: u32, seed: u64) -> Result<(), String> {
    let _ = (m, sb, cases, seed);
    Err("validate_trace: not implemented yet (OWNER W3-VERIF), the trace was not validated".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;

    #[test]
    fn the_skeleton_rejects_every_trace() {
        let m = Machine::new();
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        let e = validate_trace(&m, &sb, 8, 1).unwrap_err();
        assert!(e.contains("W3-VERIF") && e.contains("not validated"));
    }
}
