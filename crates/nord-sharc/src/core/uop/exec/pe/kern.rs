//! Compute ops executed through the exact kernels of `kernels/` on the borrowed elements of
//! `elem.rs`: the fast counterpart of [`super::exec_both`] (which runs `Pe::exec` and is the
//! oracle). One method per op of an element, the paired forms of add, subtract, multiply and
//! average for SIMD mode, and [`exec_kernels`] which runs a decoded op on PEx and, in SIMD mode,
//! on PEy, raising the arithmetic-exception bits of PEx first like the interpreter.
//!
//! OWNER: W1-KERN. The float handlers of W2-FLOAT call the methods (one per `UopKind`), or
//! [`exec_kernels`] for a kind that decodes the op at run time.
//!
//! Operand order and write order follow the interpreter: every operand is read before the result
//! is written (so `Fn = Fn + Fm` is fine), the compare shifts CACC before its flags are applied,
//! and `FIX`/`TRUNC` write an integer (extension dropped) while every other result is a float.
//!
//! Provenance: VERIFIED equal to the interpreter and to `Pe::exec` (value, hidden tag and low
//! byte, flags, ASTAT, STKY, latched interrupts) by `kernel_ops_equal_interpreter_and_pe_exec`
//! in `diff.rs`.
use super::elem::{elems, Elem};
use crate::core::decode_cache::ComputeOp;
use crate::core::uop::kernels::add::{fadd, fadd2, fsub, fsub2};
use crate::core::uop::kernels::avg::{favg, favg2};
use crate::core::uop::kernels::compare::{fcmp, fgreater};
use crate::core::uop::kernels::convert::{ffix, ffloat, frnd, fscalb};
use crate::core::uop::kernels::mul::{fmul, fmul2};
use crate::core::uop::kernels::select::{fselect, fsign};
use crate::core::uop::kernels::FlagsOut;
use crate::core::Core;

/// Whether [`exec_kernels`] implements `op`: float multiply and the float ALU opcodes
/// `0x81` add, `0x82` subtract, `0x89` average, `0x8A` compare, `0xA1` pass, `0xA2` negate,
/// `0xB0` abs, `0xA5` round, `0xBD` scale, `0xC9`/`0xD9` fix, `0xCD`/`0xDD` truncate, `0xCA`/`0xDA`
/// float and `0xE0..=0xE3` copysign, min, max, clip.
/// VERIFIED: the opcode numbers are those of `Core::float_alu` (`float_ops.rs`), checked by
/// `kernel_ops_equal_interpreter_and_pe_exec`, which runs every one of them.
pub(in crate::core) fn kernels_support(op: ComputeOp) -> bool {
    match op {
        ComputeOp::FloatMul { .. } => true,
        ComputeOp::FloatAlu { op, .. } => matches!(
            op,
            0x81 | 0x82
                | 0x89
                | 0x8A
                | 0xA1
                | 0xA2
                | 0xB0
                | 0xA5
                | 0xBD
                | 0xC9
                | 0xCA
                | 0xCD
                | 0xD9
                | 0xDA
                | 0xDD
                | 0xE0..=0xE3
        ),
        _ => false,
    }
}

impl Elem<'_> {
    /// `Fn = Fx + Fy` or `Fx - Fy`.
    #[inline(always)]
    pub fn add(&mut self, n: u8, x: u8, y: u8, sub: bool) {
        let (a, b, mode, mut st) = (self.f40(x), self.f40(y), self.mode(), FlagsOut::default());
        let v = if sub {
            fsub(a, b, mode, &mut st)
        } else {
            fadd(a, b, mode, &mut st)
        };
        self.set_f40(n, v);
        self.alu_flags(&st);
    }

    /// `Fn = Fx * Fy`.
    #[inline(always)]
    pub fn mul(&mut self, n: u8, x: u8, y: u8) {
        let (a, b, mode, mut st) = (self.f40(x), self.f40(y), self.mode(), FlagsOut::default());
        let v = fmul(a, b, mode, &mut st);
        self.set_f40(n, v);
        self.mul_flags(&st);
    }

    /// `Fn = (Fx + Fy) / 2`.
    #[inline(always)]
    pub fn avg(&mut self, n: u8, x: u8, y: u8) {
        let (a, b, mode, mut st) = (self.f40(x), self.f40(y), self.mode(), FlagsOut::default());
        let v = favg(a, b, mode, &mut st);
        self.set_f40(n, v);
        self.alu_flags(&st);
    }

    /// `COMP(Fx, Fy)`: CACC and the flags, no result register.
    #[inline(always)]
    pub fn cmp(&mut self, x: u8, y: u8) {
        let (a, b, mode, mut st) = (self.f40(x), self.f40(y), self.mode(), FlagsOut::default());
        self.shift_cacc(fgreater(a, b, mode));
        fcmp(a, b, mode, &mut st);
        self.alu_flags(&st);
    }

    /// `Fn = PASS Fx` (`kind` 0), `-Fx` (1) or `ABS Fx` (2).
    #[inline(always)]
    pub fn sign(&mut self, n: u8, x: u8, kind: u8) {
        let (a, mode, mut st) = (self.f40(x), self.mode(), FlagsOut::default());
        let v = fsign(a, kind, mode, &mut st);
        self.set_f40(n, v);
        self.alu_flags(&st);
    }

    /// `Fn = COPYSIGN`, `MIN`, `MAX` or `CLIP` (`kind` 0..3) of `Fx` and `Fy`.
    #[inline(always)]
    pub fn select(&mut self, n: u8, x: u8, y: u8, kind: u8) {
        let (a, b, mode, mut st) = (self.f40(x), self.f40(y), self.mode(), FlagsOut::default());
        let v = fselect(a, b, kind, mode, &mut st);
        self.set_f40(n, v);
        self.alu_flags(&st);
    }

    /// `Fn = FLOAT Rx BY scale`.
    #[inline(always)]
    pub fn float(&mut self, n: u8, x: u8, scale: i32) {
        let (ix, mode, mut st) = (self.int(x), self.mode(), FlagsOut::default());
        let v = ffloat(ix, scale, mode, &mut st);
        self.set_f40(n, v);
        self.alu_flags(&st);
    }

    /// `Rn = FIX Fx BY scale`, or `TRUNC` with `toward_zero`.
    #[inline(always)]
    pub fn fix(&mut self, n: u8, x: u8, scale: i32, toward_zero: bool) {
        let (a, mode, mut st) = (self.f40(x), self.mode(), FlagsOut::default());
        let v = ffix(a, scale, toward_zero, mode, &mut st);
        self.set_int(n, v as u32);
        self.alu_flags(&st);
    }

    /// `Fn = RND Fx`.
    #[inline(always)]
    pub fn rnd(&mut self, n: u8, x: u8) {
        let (a, mode, mut st) = (self.f40(x), self.mode(), FlagsOut::default());
        let v = frnd(a, mode, &mut st);
        self.set_f40(n, v);
        self.alu_flags(&st);
    }

    /// `Fn = SCALB Fx BY exp`.
    #[inline(always)]
    pub fn scalb(&mut self, n: u8, x: u8, exp: i32) {
        let (a, mode, mut st) = (self.f40(x), self.mode(), FlagsOut::default());
        let v = fscalb(a, exp, mode, &mut st);
        self.set_f40(n, v);
        self.alu_flags(&st);
    }

    /// Runs one op of [`kernels_support`] on this element; any other op does nothing.
    #[inline(always)]
    pub fn run(&mut self, op: ComputeOp) {
        match op {
            ComputeOp::FloatMul { n, x, y } => self.mul(n, x, y),
            ComputeOp::FloatAlu { op, n, x, y } => match op {
                0x81 => self.add(n, x, y, false),
                0x82 => self.add(n, x, y, true),
                0x89 => self.avg(n, x, y),
                0x8A => self.cmp(x, y),
                0xA1 => self.sign(n, x, 0),
                0xA2 => self.sign(n, x, 1),
                0xB0 => self.sign(n, x, 2),
                0xA5 => self.rnd(n, x),
                0xBD => self.scalb(n, x, self.int(y)),
                // FIX, TRUNC, FLOAT: bit 4 of the opcode selects the `BY Ry` scale form and
                // bit 2 selects TRUNC, so each kernel is expanded once, not once per opcode
                0xC9 | 0xCD | 0xD9 | 0xDD => {
                    let scale = if op & 0x10 != 0 { self.int(y) } else { 0 };
                    self.fix(n, x, scale, op & 4 != 0);
                }
                0xCA | 0xDA => {
                    let scale = if op & 0x10 != 0 { self.int(y) } else { 0 };
                    self.float(n, x, scale);
                }
                0xE0..=0xE3 => self.select(n, x, y, op & 3),
                _ => {}
            },
            _ => {}
        }
    }
}

/// `Fn = Fx + Fy` / `Fx - Fy` on both elements through the paired kernel (SIMD mode).
#[inline(always)]
pub(in crate::core) fn add2(ex: &mut Elem<'_>, ey: &mut Elem<'_>, n: u8, x: u8, y: u8, sub: bool) {
    let mode = ex.mode();
    let (a, b) = ([ex.f40(x), ey.f40(x)], [ex.f40(y), ey.f40(y)]);
    let mut st = [FlagsOut::default(); 2];
    let v = if sub {
        fsub2(a, b, mode, &mut st)
    } else {
        fadd2(a, b, mode, &mut st)
    };
    ex.set_f40(n, v[0]);
    ex.alu_flags(&st[0]);
    ey.set_f40(n, v[1]);
    ey.alu_flags(&st[1]);
}

/// `Fn = Fx * Fy` on both elements through the paired kernel.
#[inline(always)]
pub(in crate::core) fn mul2(ex: &mut Elem<'_>, ey: &mut Elem<'_>, n: u8, x: u8, y: u8) {
    let mode = ex.mode();
    let (a, b) = ([ex.f40(x), ey.f40(x)], [ex.f40(y), ey.f40(y)]);
    let mut st = [FlagsOut::default(); 2];
    let v = fmul2(a, b, mode, &mut st);
    ex.set_f40(n, v[0]);
    ex.mul_flags(&st[0]);
    ey.set_f40(n, v[1]);
    ey.mul_flags(&st[1]);
}

/// `Fn = (Fx + Fy) / 2` on both elements through the paired kernel.
#[inline(always)]
pub(in crate::core) fn avg2(ex: &mut Elem<'_>, ey: &mut Elem<'_>, n: u8, x: u8, y: u8) {
    let mode = ex.mode();
    let (a, b) = ([ex.f40(x), ey.f40(x)], [ex.f40(y), ey.f40(y)]);
    let mut st = [FlagsOut::default(); 2];
    let v = favg2(a, b, mode, &mut st);
    ex.set_f40(n, v[0]);
    ex.alu_flags(&st[0]);
    ey.set_f40(n, v[1]);
    ey.alu_flags(&st[1]);
}

/// Runs `op` on PEx and, when `simd`, on PEy through the kernels, then raises the accumulated
/// arithmetic-exception bits (PEx first). Returns false (and runs nothing) when the kernels do
/// not implement `op` (see [`kernels_support`]).
#[inline(always)]
pub(in crate::core) fn exec_kernels(core: &mut Core, op: ComputeOp, simd: bool) -> bool {
    if !kernels_support(op) {
        return false;
    }
    let (mut ex, mut ey) = elems(core);
    match (simd, op) {
        (true, ComputeOp::FloatMul { n, x, y }) => mul2(&mut ex, &mut ey, n, x, y),
        (true, ComputeOp::FloatAlu { op: 0x81, n, x, y }) => add2(&mut ex, &mut ey, n, x, y, false),
        (true, ComputeOp::FloatAlu { op: 0x82, n, x, y }) => add2(&mut ex, &mut ey, n, x, y, true),
        (true, ComputeOp::FloatAlu { op: 0x89, n, x, y }) => avg2(&mut ex, &mut ey, n, x, y),
        _ => {
            ex.run(op);
            if simd {
                ey.run(op);
            }
        }
    }
    let (sx, sy) = (ex.sticky(), ey.sticky());
    core.aot_pe_end(false, sx);
    if simd {
        core.aot_pe_end(true, sy);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    fn core_with_floats() -> Core {
        let mut c = core_with(&[0x0001]);
        // R1 = 1.5, R2 = 2.0 (and S1 = -3.0, S2 = 0.5 for PEy)
        (c.r[1], c.r[2]) = (1.5f32.to_bits(), 2.0f32.to_bits());
        (c.s[1], c.s[2]) = ((-3.0f32).to_bits(), 0.5f32.to_bits());
        c
    }

    fn add_op(op: u8) -> ComputeOp {
        ComputeOp::FloatAlu {
            op,
            n: 3,
            x: 1,
            y: 2,
        }
    }

    #[test]
    fn exec_kernels_runs_one_element_or_both_and_reports_what_it_did() {
        let mut c = core_with_floats();
        assert!(exec_kernels(&mut c, add_op(0x81), false));
        assert_eq!(f32::from_bits(c.r[3]), 3.5);
        assert_eq!(c.s[3], 0, "PEy untouched without SIMD");
        assert!(exec_kernels(&mut c, add_op(0x81), true));
        assert_eq!(f32::from_bits(c.s[3]), -2.5);
        assert!(exec_kernels(
            &mut c,
            ComputeOp::FloatMul { n: 4, x: 1, y: 2 },
            true
        ));
        assert_eq!(
            (f32::from_bits(c.r[4]), f32::from_bits(c.s[4])),
            (3.0, -1.5)
        );
        assert!(
            c.sys.astat_extra[1] >> 6 & 1 == 1,
            "PEy product is negative: MN"
        );
        assert!(exec_kernels(&mut c, add_op(0x89), true));
        assert_eq!(
            (f32::from_bits(c.r[3]), f32::from_bits(c.s[3])),
            (1.75, -1.25)
        );
    }

    #[test]
    fn an_op_the_kernels_do_not_implement_runs_nothing() {
        let mut c = core_with_floats();
        let before = (c.r, c.s, c.flags, c.sys.astat_extra);
        for op in [
            ComputeOp::Nop,
            ComputeOp::Invalid,
            ComputeOp::FixedAlu {
                op: 0x01,
                n: 3,
                x: 1,
                y: 2,
            },
            add_op(0xC4), // RECIPS: seeds are not a kernel
            add_op(0x91), // |Fx + Fy|
            add_op(0xF3),
        ] {
            assert!(!kernels_support(op), "{op:?}");
            assert!(!exec_kernels(&mut c, op, true), "{op:?}");
        }
        assert_eq!(before, (c.r, c.s, c.flags, c.sys.astat_extra));
    }

    #[test]
    fn the_result_register_may_alias_an_operand() {
        let mut c = core_with_floats();
        // F1 = F1 + F1 and F2 = F1 - F2: both operands are read before the result is written
        assert!(exec_kernels(
            &mut c,
            ComputeOp::FloatAlu {
                op: 0x81,
                n: 1,
                x: 1,
                y: 1
            },
            false
        ));
        assert_eq!(f32::from_bits(c.r[1]), 3.0);
        assert!(exec_kernels(
            &mut c,
            ComputeOp::FloatAlu {
                op: 0x82,
                n: 2,
                x: 1,
                y: 2
            },
            false
        ));
        assert_eq!(f32::from_bits(c.r[2]), 1.0);
    }
}
