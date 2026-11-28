//! Instruction-count probes of the kernels: one `#[inline(never)]` function per kernel whose
//! symbol `uop_probe_k_*` `tools/uop_insn_count.py` finds in the compiler assembly of the test
//! build (`--emit=asm`) and counts. The kernels are `#[inline(always)]` into the probe, so the probe body is the
//! instruction sequence a handler pays for the kernel (cold paths stay out of line and show as
//! `call`s). Owner-run for gates K1 and K2 (design 12); not a unit test, because Rust has no
//! in-process instruction counter.
//!
//! OWNER: W1-KERN, test build only. The test below calls every probe through `black_box`, which
//! keeps the symbols in the test binary and checks each probe against the kernel it wraps.
use super::add::{fadd, fadd2, fsub, fsub2};
use super::avg::{favg, favg2};
use super::compare::fcmp;
use super::convert::{ffix, ffloat, frnd, fscalb};
use super::mul::{fmul, fmul2};
use super::select::{fselect, fsign};
use super::{FlagsOut, Mode, F40};

macro_rules! binary_probe {
    ($name:ident, $kernel:path) => {
        #[inline(never)]
        pub(crate) fn $name(a: F40, b: F40, m1: u32, st: &mut FlagsOut) -> F40 {
            $kernel(a, b, Mode::from_mode1(m1), st)
        }
    };
}

macro_rules! pair_probe {
    ($name:ident, $kernel:path) => {
        #[inline(never)]
        pub(crate) fn $name(a: [F40; 2], b: [F40; 2], m1: u32, st: &mut [FlagsOut; 2]) -> [F40; 2] {
            $kernel(a, b, Mode::from_mode1(m1), st)
        }
    };
}

binary_probe!(uop_probe_k_fadd, fadd);
binary_probe!(uop_probe_k_fsub, fsub);
binary_probe!(uop_probe_k_fmul, fmul);
binary_probe!(uop_probe_k_favg, favg);
binary_probe!(uop_probe_k_fcmp, fcmp);
pair_probe!(uop_probe_k_fadd2, fadd2);
pair_probe!(uop_probe_k_fsub2, fsub2);
pair_probe!(uop_probe_k_fmul2, fmul2);
pair_probe!(uop_probe_k_favg2, favg2);

#[inline(never)]
pub(crate) fn uop_probe_k_fselect(a: F40, b: F40, kind: u8, m1: u32, st: &mut FlagsOut) -> F40 {
    fselect(a, b, kind, Mode::from_mode1(m1), st)
}

#[inline(never)]
pub(crate) fn uop_probe_k_fsign(a: F40, kind: u8, m1: u32, st: &mut FlagsOut) -> F40 {
    fsign(a, kind, Mode::from_mode1(m1), st)
}

#[inline(never)]
pub(crate) fn uop_probe_k_ffloat(x: i32, scale: i32, m1: u32, st: &mut FlagsOut) -> F40 {
    ffloat(x, scale, Mode::from_mode1(m1), st)
}

#[inline(never)]
pub(crate) fn uop_probe_k_ffix(a: F40, scale: i32, tz: bool, m1: u32, st: &mut FlagsOut) -> F40 {
    ffix(a, scale, tz, Mode::from_mode1(m1), st)
}

#[inline(never)]
pub(crate) fn uop_probe_k_frnd(a: F40, m1: u32, st: &mut FlagsOut) -> F40 {
    frnd(a, Mode::from_mode1(m1), st)
}

#[inline(never)]
pub(crate) fn uop_probe_k_fscalb(a: F40, n: i32, m1: u32, st: &mut FlagsOut) -> F40 {
    fscalb(a, n, Mode::from_mode1(m1), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::kernels::f40;
    use std::hint::black_box as bb;

    #[test]
    fn every_probe_returns_what_its_kernel_returns() {
        let (a, b, m1) = (bb(f40(3.5)), bb(f40(-1.25)), bb(0u32));
        let mode = Mode::from_mode1(m1);
        let (mut p, mut k) = (FlagsOut::default(), FlagsOut::default());
        type Binary = fn(F40, F40, u32, &mut FlagsOut) -> F40;
        type BinaryKernel = fn(F40, F40, Mode, &mut FlagsOut) -> F40;
        let binary: [(Binary, BinaryKernel); 5] = [
            (uop_probe_k_fadd, fadd),
            (uop_probe_k_fsub, fsub),
            (uop_probe_k_fmul, fmul),
            (uop_probe_k_favg, favg),
            (uop_probe_k_fcmp, fcmp),
        ];
        for (probe, kernel) in binary {
            assert_eq!(probe(a, b, m1, &mut p), kernel(a, b, mode, &mut k));
            assert_eq!(p, k);
        }
        let (mut p, mut k) = ([FlagsOut::default(); 2], [FlagsOut::default(); 2]);
        let (a2, b2) = ([a, bb(f40(7.0))], [b, bb(f40(0.5))]);
        type Pair = fn([F40; 2], [F40; 2], u32, &mut [FlagsOut; 2]) -> [F40; 2];
        type PairKernel = fn([F40; 2], [F40; 2], Mode, &mut [FlagsOut; 2]) -> [F40; 2];
        let pairs: [(Pair, PairKernel); 4] = [
            (uop_probe_k_fadd2, fadd2),
            (uop_probe_k_fsub2, fsub2),
            (uop_probe_k_fmul2, fmul2),
            (uop_probe_k_favg2, favg2),
        ];
        for (probe, kernel) in pairs {
            assert_eq!(probe(a2, b2, m1, &mut p), kernel(a2, b2, mode, &mut k));
            assert_eq!(p, k);
        }
        let (mut p, mut k) = (FlagsOut::default(), FlagsOut::default());
        assert_eq!(
            uop_probe_k_fselect(a, b, bb(2), m1, &mut p),
            fselect(a, b, 2, mode, &mut k)
        );
        assert_eq!(p, k);
        assert_eq!(
            uop_probe_k_fsign(a, bb(2), m1, &mut p),
            fsign(a, 2, mode, &mut k)
        );
        assert_eq!(
            uop_probe_k_ffloat(bb(-9), bb(1), m1, &mut p),
            ffloat(-9, 1, mode, &mut k)
        );
        assert_eq!(p, k);
        assert_eq!(
            uop_probe_k_ffix(a, bb(0), bb(true), m1, &mut p),
            ffix(a, 0, true, mode, &mut k)
        );
        assert_eq!(uop_probe_k_frnd(a, m1, &mut p), frnd(a, mode, &mut k));
        assert_eq!(
            uop_probe_k_fscalb(a, bb(3), m1, &mut p),
            fscalb(a, 3, mode, &mut k)
        );
        assert_eq!(p, k);
        assert_eq!(uop_probe_k_fadd(a, b, m1, &mut p), f40(2.25), "3.5 + -1.25");
    }
}
