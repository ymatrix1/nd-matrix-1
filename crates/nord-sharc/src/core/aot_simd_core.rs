//! The SIMD-mode compute form on [`Core`]: `Fn = Fx + Fy`, `Fx - Fy` and `Fx * Fy` executed on
//! PEx and PEy together through the paired kernels of `aot_simd.rs`. [`Core::aot_pair_exec`] reads
//! the operands of both elements, runs one paired kernel, and writes the result registers, flags,
//! multiplier bits and arithmetic-exception bits of each element in place (no PE swap), PEx first
//! like the interpreter, so every effect equals running the op on each element in turn.
//!
//! Provenance: VERIFIED equal to the interpreter's `exec_compute_op` run on PEx then PEy, for
//! register values, float extensions, ALU and multiplier flags, ASTAT bits and STKY bits, by
//! `pair_exec_equals_the_interpreter_on_both_elements` (random state, every rounding mode).
// Called from the lowering once it takes the paired form; until then only the tests use it.
#![allow(dead_code)]

use super::aot_simd::{fast_add2, fast_mul2};
use super::decode_cache::ComputeOp;
use super::float::{Mode, Out};
use super::Core;

/// ASTAT bit 10: the last ALU operation was floating-point.
const AF: u32 = 1 << 10;

impl Core {
    /// Whether [`Core::aot_pair_exec`] implements `op` (float add, subtract and multiply).
    pub(super) fn aot_pairable(op: ComputeOp) -> bool {
        matches!(
            op,
            ComputeOp::FloatAlu {
                op: 0x81 | 0x82,
                ..
            } | ComputeOp::FloatMul { .. }
        )
    }

    /// Runs `op` on both elements with the paired kernel; false (nothing done) when the op is not
    /// one [`Core::aot_pairable`] lists. Call it only in SIMD mode.
    #[inline(always)]
    pub(super) fn aot_pair_exec(&mut self, op: ComputeOp) -> bool {
        let (n, x, y, kind) = match op {
            ComputeOp::FloatAlu { op: 0x81, n, x, y } => (n, x, y, Kind::Add),
            ComputeOp::FloatAlu { op: 0x82, n, x, y } => (n, x, y, Kind::Sub),
            ComputeOp::FloatMul { n, x, y } => (n, x, y, Kind::Mul),
            _ => return false,
        };
        let mode = Mode::from_mode1(self.mode1);
        let a = [self.f40_of::<false>(x), self.f40_of::<true>(x)];
        let b = [self.f40_of::<false>(y), self.f40_of::<true>(y)];
        let o = match kind {
            Kind::Add => fast_add2(a, b, false, mode),
            Kind::Sub => fast_add2(a, b, true, mode),
            Kind::Mul => fast_mul2(a, b, mode),
        };
        let mul = kind == Kind::Mul;
        let sticky = self.pair_write::<false>(n, o[0], mul);
        self.aot_pe_end(false, sticky);
        let sticky = self.pair_write::<true>(n, o[1], mul);
        self.aot_pe_end(true, sticky);
        true
    }

    /// The 40-bit float in register `n` of PEx (`Y` false) or PEy.
    #[inline(always)]
    fn f40_of<const Y: bool>(&self, n: u8) -> u64 {
        let (r, fx) = if Y {
            (&self.s, &self.fx_y)
        } else {
            (&self.r, &self.fx)
        };
        let n = usize::from(n);
        let lo = if fx.tag[n] == r[n] { fx.lo[n] } else { 0 };
        u64::from(r[n]) << 8 | u64::from(lo)
    }

    /// Writes the result of PEx (`Y` false) or PEy and flags like `float_result` / `mul_result`; returns the
    /// arithmetic-exception bits to raise.
    #[inline(always)]
    fn pair_write<const Y: bool>(&mut self, n: u8, o: Out, mul: bool) -> u32 {
        let (r, fx, f) = if Y {
            (&mut self.s, &mut self.fx_y, &mut self.flags_y)
        } else {
            (&mut self.r, &mut self.fx, &mut self.flags)
        };
        let (n, astat) = (usize::from(n), &mut self.sys.astat_extra[usize::from(Y)]);
        r[n] = (o.v >> 8) as u32;
        fx.tag[n] = r[n];
        fx.lo[n] = o.v as u8;
        if mul {
            let m = u32::from(o.mn)
                | u32::from(o.mv) << 1
                | u32::from(o.mu) << 2
                | u32::from(o.mi) << 3;
            *astat = *astat & !(0xF << 6) | m << 6;
            return u32::from(o.mv) << 7 | u32::from(o.mu) << 8 | u32::from(o.mi) << 9;
        }
        (f.az, f.an, f.av, f.ac, f.as_, f.ai) = (o.az, o.an, o.av, false, o.as_, o.ai);
        *astat |= AF;
        u32::from(o.au) | u32::from(o.av) << 1 | u32::from(o.ai) << 5
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Add,
    Sub,
    Mul,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// A 40-bit float: mostly ordinary, sometimes zero, denormal, infinity, near the range edges,
    /// and often with a set extension.
    fn operand(r: &mut Rng) -> u64 {
        let v = r.next();
        let exp = match (v >> 40) % 12 {
            0 => 0,
            1 => 255,
            2 => 1 + r.next() % 3,
            3 => 252 + r.next() % 3,
            _ => 100 + r.next() % 55,
        };
        (v >> 63) << 39 | exp << 31 | (v >> 8) & 0x7FFF_FFFF | v & 0xFF
    }

    #[test]
    fn pair_exec_equals_the_interpreter_on_both_elements() {
        let ops = [
            ComputeOp::FloatAlu {
                op: 0x81,
                n: 3,
                x: 1,
                y: 2,
            },
            ComputeOp::FloatAlu {
                op: 0x82,
                n: 4,
                x: 2,
                y: 1,
            },
            ComputeOp::FloatMul { n: 8, x: 1, y: 2 },
            ComputeOp::FloatMul { n: 1, x: 1, y: 1 },
            ComputeOp::FloatAlu {
                op: 0x81,
                n: 2,
                x: 2,
                y: 2,
            },
        ];
        let mut r = Rng(0xFEED_FACE_0123_4567);
        for case in 0..6000 {
            let mut slow = Core::new(&[]);
            slow.mode1 = super::super::MODE1_PEYEN | [0, 1 << 15, 1 << 16, 3 << 15][case % 4];
            for k in 0..16 {
                slow.r[k] = (operand(&mut r) >> 8) as u32;
                slow.s[k] = (operand(&mut r) >> 8) as u32;
                slow.fx.tag[k] = slow.r[k];
                slow.fx.lo[k] = r.next() as u8;
                slow.fx_y.tag[k] = slow.s[k];
                slow.fx_y.lo[k] = r.next() as u8;
            }
            let mut fast = Core::new(&[]);
            (fast.mode1, fast.r, fast.s) = (slow.mode1, slow.r, slow.s);
            (fast.fx.tag, fast.fx.lo) = (slow.fx.tag, slow.fx.lo);
            (fast.fx_y.tag, fast.fx_y.lo) = (slow.fx_y.tag, slow.fx_y.lo);
            for op in ops {
                assert!(Core::aot_pairable(op));
                assert!(slow.exec_compute_op(op));
                slow.swap_pe();
                assert!(slow.exec_compute_op(op));
                slow.swap_pe();
                assert!(fast.aot_pair_exec(op));
                assert_eq!(slow.state_mismatch(&fast), None, "case {case} {op:?}");
            }
        }
    }

    #[test]
    fn other_ops_are_not_paired() {
        let op = ComputeOp::FloatAlu {
            op: 0xDA,
            n: 1,
            x: 2,
            y: 3,
        };
        assert!(!Core::aot_pairable(op));
        assert!(!Core::new(&[]).aot_pair_exec(op));
        assert!(!Core::aot_pairable(ComputeOp::Nop));
    }
}
