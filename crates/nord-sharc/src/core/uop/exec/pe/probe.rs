//! Instruction-count probes of the element ops: one `#[inline(never)]` function per op whose
//! symbol `uop_probe_e_*` `tools/uop_insn_count.py` finds in the compiler assembly of the test
//! build (`--emit=asm`) and counts. Each probe is what a float handler does for the op: borrow the element(s), run
//! the kernel, write the result and flags, raise the sticky bits (`Core::aot_pe_end`, a cold
//! call when a bit is set). The kernels are `#[inline(always)]`, so the probe body is the handler
//! body; cold paths of the kernels stay out of line and show as `call`s.
//!
//! OWNER: W1-KERN, test build only. Owner-run for gates K1 and K2 (design 12); not a unit test,
//! because Rust has no in-process instruction counter. The tests below run every probe through
//! `black_box` (which keeps the symbols in the test binary) and check each against the dispatch
//! `exec_kernels` on a second core, on one element and on both.
use super::{add2, avg2, elems, exec_kernels, mul2};
use crate::core::decode_cache::ComputeOp;
use crate::core::Core;

/// One element probe: borrow PEx, run the `Elem` method `$method` on it, raise its sticky bits.
/// `$extra` are the constant trailing arguments of the method (a kind, a sub flag).
macro_rules! element_probe {
    ($name:ident, $method:ident, ($($arg:ident: $t:ty),*) $(, $extra:expr)*) => {
        #[inline(never)]
        pub(in crate::core) fn $name(core: &mut Core, $($arg: $t),*) {
            let (mut e, _) = elems(core);
            e.$method($($arg,)* $($extra),*);
            let s = e.sticky();
            core.aot_pe_end(false, s);
        }
    };
}

/// One paired probe: borrow both elements, run the paired kernel, raise both sticky words.
macro_rules! pair_probe {
    ($name:ident, $paired:ident $(, $extra:expr)*) => {
        #[inline(never)]
        pub(in crate::core) fn $name(core: &mut Core, n: u8, x: u8, y: u8) {
            let (mut ex, mut ey) = elems(core);
            $paired(&mut ex, &mut ey, n, x, y $(, $extra)*);
            let (sx, sy) = (ex.sticky(), ey.sticky());
            core.aot_pe_end(false, sx);
            core.aot_pe_end(true, sy);
        }
    };
}

element_probe!(uop_probe_e_fadd, add, (n: u8, x: u8, y: u8), false);
element_probe!(uop_probe_e_fsub, add, (n: u8, x: u8, y: u8), true);
element_probe!(uop_probe_e_fmul, mul, (n: u8, x: u8, y: u8));
element_probe!(uop_probe_e_favg, avg, (n: u8, x: u8, y: u8));
element_probe!(uop_probe_e_fcmp, cmp, (x: u8, y: u8));
element_probe!(uop_probe_e_fabs, sign, (n: u8, x: u8), 2);
element_probe!(uop_probe_e_fselect, select, (n: u8, x: u8, y: u8), 2);
element_probe!(uop_probe_e_ffloat, float, (n: u8, x: u8, s: i32));
element_probe!(uop_probe_e_ffix, fix, (n: u8, x: u8, s: i32), false);
element_probe!(uop_probe_e_frnd, rnd, (n: u8, x: u8));
element_probe!(uop_probe_e_fscalb, scalb, (n: u8, x: u8, s: i32));

pair_probe!(uop_probe_e_fadd2, add2, false);
pair_probe!(uop_probe_e_fsub2, add2, true);
pair_probe!(uop_probe_e_fmul2, mul2);
pair_probe!(uop_probe_e_favg2, avg2);

/// The run-time dispatch `exec_kernels` (a handler that decodes the op itself).
#[inline(never)]
pub(in crate::core) fn uop_probe_e_exec_kernels(
    core: &mut Core,
    op: ComputeOp,
    simd: bool,
) -> bool {
    exec_kernels(core, op, simd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::verify::compare::exact_diff;
    use std::hint::black_box as bb;

    fn core() -> Core {
        let mut c = core_with(&[0x0001]);
        (c.r[1], c.r[2], c.s[1], c.s[2]) = (
            1.5f32.to_bits(),
            (-2.25f32).to_bits(),
            7.0f32.to_bits(),
            0.125f32.to_bits(),
        );
        c
    }

    /// Runs `probe` on one core and `op` through `exec_kernels` on another: the two end equal,
    /// and the probe did change the core (so equal is not the trivial "nothing ran").
    fn agree(probe: impl Fn(&mut Core), op: ComputeOp, simd: bool) {
        let (mut p, mut k) = (core(), core());
        probe(&mut p);
        assert!(uop_probe_e_exec_kernels(&mut k, bb(op), bb(simd)));
        assert_eq!(exact_diff(&p, &k), None, "{op:?} simd {simd}");
        assert_ne!(
            exact_diff(&p, &core()),
            None,
            "{op:?} simd {simd} changed nothing"
        );
    }

    fn alu(op: u8) -> ComputeOp {
        ComputeOp::FloatAlu {
            op,
            n: 3,
            x: 1,
            y: 2,
        }
    }

    #[test]
    fn every_element_probe_runs_the_op_it_names() {
        let (n, x, y) = (bb(3u8), bb(1u8), bb(2u8));
        agree(|c| uop_probe_e_fadd(c, n, x, y), alu(0x81), false);
        agree(|c| uop_probe_e_fsub(c, n, x, y), alu(0x82), false);
        agree(|c| uop_probe_e_favg(c, n, x, y), alu(0x89), false);
        agree(|c| uop_probe_e_fcmp(c, x, y), alu(0x8A), false);
        agree(|c| uop_probe_e_fabs(c, n, x), alu(0xB0), false);
        agree(|c| uop_probe_e_fselect(c, n, x, y), alu(0xE2), false);
        agree(|c| uop_probe_e_frnd(c, n, x), alu(0xA5), false);
        agree(|c| uop_probe_e_ffloat(c, n, x, bb(0)), alu(0xCA), false);
        agree(|c| uop_probe_e_ffix(c, n, x, bb(0)), alu(0xC9), false);
        agree(
            |c| {
                // the scale operand of SCALB is R2 as an integer
                let scale = c.r[2] as i32;
                uop_probe_e_fscalb(c, n, x, bb(scale));
            },
            alu(0xBD),
            false,
        );
        agree(
            |c| uop_probe_e_fmul(c, n, x, y),
            ComputeOp::FloatMul { n: 3, x: 1, y: 2 },
            false,
        );
    }

    #[test]
    fn every_paired_probe_equals_the_dispatch_on_both_elements() {
        let (n, x, y) = (bb(3u8), bb(1u8), bb(2u8));
        agree(|c| uop_probe_e_fadd2(c, n, x, y), alu(0x81), true);
        agree(|c| uop_probe_e_fsub2(c, n, x, y), alu(0x82), true);
        agree(|c| uop_probe_e_favg2(c, n, x, y), alu(0x89), true);
        agree(
            |c| uop_probe_e_fmul2(c, n, x, y),
            ComputeOp::FloatMul { n: 3, x: 1, y: 2 },
            true,
        );
    }

    #[test]
    fn a_paired_probe_writes_the_other_element_and_a_single_probe_does_not() {
        let (mut one, mut two) = (core(), core());
        uop_probe_e_fadd(&mut one, bb(3), bb(1), bb(2));
        uop_probe_e_fadd2(&mut two, bb(3), bb(1), bb(2));
        assert_eq!(f32::from_bits(one.r[3]), -0.75);
        assert_eq!(one.s[3], 0, "PEy untouched by the one-element probe");
        assert_eq!(f32::from_bits(two.s[3]), 7.125);
    }
}
