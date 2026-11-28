//! In-place access to PEx and PEy for the compute handlers, and the reference execution used as
//! the oracle of the kernels.
//!
//! OWNER: W1-KERN. Three parts, one file each (design P11):
//!
//! | File | Content |
//! |---|---|
//! | `elem.rs` | [`Elem`], one element borrowed in place (registers, float extensions, flags, ASTAT bits, pending STKY), and [`elems`], PEx and PEy together |
//! | `kern.rs` | the ops on [`Elem`] through the exact kernels of `kernels/`, the paired SIMD forms and [`exec_kernels`] |
//! | this file | [`exec_both`], the reference execution of one pre-decoded compute op on both elements through the v1 `Pe::exec` (design 4.7: the semantics of `FGen` / `XGen` are `Pe::exec`), which stays the oracle |
//!
//! `diff.rs` (test build) runs `exec_kernels` against the interpreter and against `exec_both` on
//! random element states and compares every field an op can write, hidden extension included.
//! `probe.rs` (test build) holds the `#[inline(never)]` instruction-count probes of
//! `tools/uop_insn_count.py`.
mod elem;
mod kern;

#[cfg(test)]
mod diff;
#[cfg(test)]
mod probe;

pub(in crate::core) use elem::{elems, Elem, AF};
pub(in crate::core) use kern::{add2, avg2, exec_kernels, kernels_support, mul2};

use crate::core::aot_pe::{pe_supports, Pe};
use crate::core::decode_cache::ComputeOp;
use crate::core::Core;

/// Runs `op` on PEx and, when `simd`, on PEy through `Pe::exec`, then raises the accumulated
/// arithmetic-exception bits. Returns false (and runs nothing) when `Pe` does not implement `op`.
pub(in crate::core) fn exec_both(core: &mut Core, op: ComputeOp, simd: bool) -> bool {
    if !pe_supports(op) {
        return false;
    }
    for y in [false, true] {
        if y && !simd {
            break;
        }
        let mut pe = Pe::load(core, y);
        pe.exec(op);
        let sticky = pe.sticky();
        core.aot_pe_end(y, sticky);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::decode_cache::DecodedOp;
    use crate::core::support::core_with;
    use crate::core::uop::verify::compare::exact_diff;

    /// R3 = R1 + R2 (type 2a, IF TRUE).
    const FIXED_ADD: [u16; 3] = [0x013e, 0x0000, 0x1312];

    fn seeded(simd: bool) -> Core {
        let mut c = core_with(&FIXED_ADD);
        (c.r[1], c.r[2], c.s[1], c.s[2]) = (4, 5, 0x7FFF_FFFF, 1);
        if simd {
            c.mode1 |= 1 << 21;
        }
        c
    }

    #[test]
    fn exec_both_equals_one_interpreter_step_on_one_and_two_elements() {
        for simd in [false, true] {
            let (mut a, mut b) = (seeded(simd), seeded(simd));
            let pc = a.pc;
            let DecodedOp::ComputeCond { op, .. } = a.fetch_and_cache(pc, false).op else {
                panic!("not a conditional compute");
            };
            assert!(exec_both(&mut a, op, simd));
            b.step().unwrap();
            // the step prologue also schedules the next event; the uop path does that at entry
            (a.pc, a.steps, a.irq.due) = (b.pc, b.steps, b.irq.due);
            assert_eq!(exact_diff(&a, &b), None, "simd {simd}");
            assert_eq!((a.r[3], a.s[3] != 0), (9, simd));
        }
    }

    #[test]
    fn an_op_pe_does_not_implement_runs_nothing() {
        let mut c = seeded(false);
        let before = c.r;
        assert!(!exec_both(&mut c, ComputeOp::Invalid, false));
        assert_eq!(c.r, before);
    }
}
