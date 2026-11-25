//! Register-resident lowering of the hot-block AOT prototype: one processing element's data
//! registers, float extensions, ALU/shifter flags, ASTAT extra bits and pending STKY bits live in
//! a local [`Pe`] for the whole block, so LLVM keeps them in registers and drops the flag stores
//! that a later instruction overwrites. [`Pe::load`] reads the element from [`Core`] once,
//! [`Pe::store`] writes it back and raises the arithmetic-exception interrupts once.
//!
//! Every method mirrors the interpreter glue it replaces (`float_ops.rs`, `fixed_alu.rs`,
//! `shifter.rs`, `multifunction.rs`) and uses the same pure kernels (`float_misc`, the verified
//! `aot_fast` kernels), so results, flags, STKY bits and latched interrupts are identical. That is
//! VERIFIED by `pe_ops_equal_interpreter_ops_on_random_simd_state` and by the real-firmware
//! differential in `docs/research/sharc-aot-hot-block.md`; the op set is what the ORG block
//! `0x125021..=0x1250aa` uses (see [`pe_supports`]).
// Only the generated block (`NORD_SHARC_AOT_HOT`) calls these; the stub build leaves them unused.
#![allow(dead_code)]

use super::aot_fast::{fast_add, fast_from_int, fast_mul};
use super::aot_native::{
    approx_add, approx_from_int, approx_mul, guarded_add, guarded_from_int, guarded_mul, Kern,
};
use super::decode_cache::ComputeOp;
use super::float::{self, Mode, Out, MODE1_ALUSAT};
use super::float_conv as conv;
use super::float_misc as misc;
use super::shifter_bits;
use super::types::Flags;
use super::Core;
use crate::visa::bits;

/// ASTAT extra bit 10 (AF): the last ALU operation was floating-point.
const AF: u32 = 1 << 10;
/// ASTAT extra bit 14, cleared by every shifter operation.
const SHIFTER_CLEAR: u32 = 1 << 14;
/// STKY bit AOS: fixed-point overflow.
const STKY_AOS: u32 = 1 << 2;
/// Float ALU opcodes of the float multifunction forms, indexed by `op6 & 7` (`multifunction.rs`).
const FLOAT_ALU_OPS: [u8; 8] = [0x81, 0x82, 0xDA, 0xD9, 0x89, 0xB0, 0xE2, 0xE1];

/// One element (PEx or PEy) of a core, worked on in place: the registers, float extensions, flags
/// and ASTAT extra bits are borrowed from the [`Core`] for the duration of a block or a single
/// kernel, so one compute op does not copy the ~170 bytes of element state in and out (that
/// copy was about half the cost of a lowered op). The pending STKY bits stay local and are
/// raised once through [`Core::aot_pe_end`].
pub(super) struct Pe<'a> {
    y: bool,
    r: &'a mut [u32; 16],
    tag: &'a mut [u32; 16],
    lo: &'a mut [u8; 16],
    f: &'a mut Flags,
    astat: &'a mut u32,
    sticky: u32,
    mode: Mode,
    sat: bool,
    kern: Kern,
}

/// Whether [`Pe::exec`] implements `op`.
pub(super) fn pe_supports(op: ComputeOp) -> bool {
    match op {
        ComputeOp::Nop | ComputeOp::FloatMul { .. } => true,
        ComputeOp::Shifter { op, .. } => pe_supports_shift(u64::from(op)),
        ComputeOp::FloatAlu { op, .. } => {
            matches!(
                op,
                0x81 | 0x82
                    | 0x89
                    | 0x8A
                    | 0xA1
                    | 0xA2
                    | 0xB0
                    | 0xC4
                    | 0xC5
                    | 0xC9
                    | 0xD9
                    | 0xDA
                    | 0xE0..=0xE3
            )
        }
        ComputeOp::FixedAlu { op, .. } => {
            matches!(op, 0x01 | 0x02 | 0x0A | 0x21 | 0x22 | 0x29 | 0x40..=0x42 | 0x61..=0x63)
        }
        ComputeOp::Multifunction { c } => float_multi(c).is_some(),
        _ => false,
    }
}

/// Whether [`Pe::shift`] implements the register-form shifter opcode `op`.
pub(super) fn pe_supports_shift(op: u64) -> bool {
    matches!(
        op,
        0x00 | 0x04
            | 0x08
            | 0x20
            | 0x24
            | 0x40
            | 0x44
            | 0x48
            | 0x4C
            | 0x64
            | 0x6C
            | 0xC0
            | 0xC4
            | 0xC8
            | 0xCC
    )
}

/// `(rm, xm, ym, alu opcode, ra, xa, ya)` of a float multiplier + float ALU multifunction word.
fn float_multi(c: u64) -> Option<(u8, u8, u8, u8, u8, u8, u8)> {
    if bits(c, 21, 17) == 0 || bits(c, 21, 21) == 1 || bits(c, 21, 19) != 3 {
        return None;
    }
    let alu = FLOAT_ALU_OPS[bits(c, 18, 16) as usize];
    Some((
        bits(c, 15, 12) as u8,
        bits(c, 7, 6) as u8,
        4 + bits(c, 5, 4) as u8,
        alu,
        bits(c, 11, 8) as u8,
        8 + bits(c, 3, 2) as u8,
        12 + bits(c, 1, 0) as u8,
    ))
}

impl<'a> Pe<'a> {
    /// Borrows element `y` (false = PEx) of `core`.
    #[inline(always)]
    pub(super) fn load(core: &'a mut Core, y: bool) -> Pe<'a> {
        let (mode, sat) = (Mode::from_mode1(core.mode1), core.mode1 & MODE1_ALUSAT != 0);
        let (r, fx, f) = if y {
            (&mut core.s, &mut core.fx_y, &mut core.flags_y)
        } else {
            (&mut core.r, &mut core.fx, &mut core.flags)
        };
        Pe {
            y,
            r,
            tag: &mut fx.tag,
            lo: &mut fx.lo,
            f,
            astat: &mut core.sys.astat_extra[usize::from(y)],
            sticky: 0,
            mode,
            sat,
            kern: Kern::Exact,
        }
    }

    /// The same element with the float kernels `kern` (a constant in generated code, so the
    /// choice folds away).
    #[inline(always)]
    pub(super) fn with_kern(mut self, kern: Kern) -> Pe<'a> {
        self.kern = kern;
        self
    }

    #[inline(always)]
    fn k_add(&self, a: u64, b: u64, sub: bool) -> Out {
        if self.kern == Kern::Approx {
            return approx_add(a, b, sub);
        }
        if self.kern == Kern::Guarded {
            if let Some(o) = guarded_add(a, b, sub, self.mode) {
                return o;
            }
        }
        fast_add(a, b, sub, self.mode)
    }

    #[inline(always)]
    fn k_mul(&self, a: u64, b: u64) -> Out {
        if self.kern == Kern::Approx {
            return approx_mul(a, b);
        }
        if self.kern == Kern::Guarded {
            if let Some(o) = guarded_mul(a, b, self.mode) {
                return o;
            }
        }
        fast_mul(a, b, self.mode)
    }

    #[inline(always)]
    fn k_from_int(&self, x: i32, scale: i32) -> Out {
        if self.kern == Kern::Approx {
            return approx_from_int(x, scale);
        }
        if self.kern == Kern::Guarded {
            if let Some(o) = guarded_from_int(x, scale) {
                return o;
            }
        }
        fast_from_int(x, scale, self.mode)
    }

    /// The arithmetic-exception bits accumulated so far; pass them to [`Core::aot_pe_end`] once the
    /// element's borrow has ended.
    #[inline(always)]
    pub(super) fn sticky(&self) -> u32 {
        self.sticky
    }

    /// The 40-bit float in register `n` (the extension counts while its tag matches).
    #[inline(always)]
    fn f40(&self, n: u8) -> u64 {
        let n = usize::from(n);
        let lo = if self.tag[n] == self.r[n] {
            self.lo[n]
        } else {
            0
        };
        u64::from(self.r[n]) << 8 | u64::from(lo)
    }

    #[inline(always)]
    fn set_f40(&mut self, n: u8, v: u64) {
        let n = usize::from(n);
        self.r[n] = (v >> 8) as u32;
        self.tag[n] = self.r[n];
        self.lo[n] = v as u8;
    }

    /// `Core::apply`: the ALU flags of a float result and the sticky bits they latch.
    #[inline(always)]
    fn apply(&mut self, o: Out) {
        (
            self.f.az, self.f.an, self.f.av, self.f.ac, self.f.as_, self.f.ai,
        ) = (o.az, o.an, o.av, false, o.as_, o.ai);
        self.sticky |= u32::from(o.au) | u32::from(o.av) << 1 | u32::from(o.ai) << 5;
    }

    #[inline(always)]
    fn float_result(&mut self, n: u8, o: Out) {
        self.set_f40(n, o.v);
        self.apply(o);
    }

    /// `Core::mul_result`: the multiplier flags MN..MI (ASTAT bits 6..9) and sticky MVS/MUS/MIS.
    #[inline(always)]
    fn mul_result(&mut self, n: u8, o: Out) {
        self.set_f40(n, o.v);
        let mult =
            u32::from(o.mn) | u32::from(o.mv) << 1 | u32::from(o.mu) << 2 | u32::from(o.mi) << 3;
        (*self.astat) = (*self.astat) & !(0xF << 6) | mult << 6;
        self.sticky |= u32::from(o.mv) << 7 | u32::from(o.mu) << 8 | u32::from(o.mi) << 9;
    }

    /// `Core::fixed_result`: `Rn = r` with the five fixed-point flags (and AI cleared).
    #[inline(always)]
    fn fixed_result(&mut self, n: u8, r: u32, ac: bool, av: bool) {
        let n = usize::from(n);
        self.r[n] = r;
        self.lo[n] = 0;
        (
            self.f.az, self.f.an, self.f.av, self.f.ac, self.f.as_, self.f.ai,
        ) = (r == 0, (r as i32) < 0, av, ac, false, false);
        if av {
            self.sticky |= STKY_AOS;
        }
    }

    /// `Core::fixed_add` / `fixed_sub` (`Rx + Ry + cin`, or `Rx + !Ry + cin`) with ALUSAT.
    #[inline(always)]
    fn fixed_addsub(&mut self, n: u8, x: u32, y: u32, cin: u64, sub: bool) {
        let (yy, ey) = if sub {
            (u64::from(!y), -i64::from(y as i32) - 1)
        } else {
            (u64::from(y), i64::from(y as i32))
        };
        let t = u64::from(x) + yy + cin;
        let exact = i64::from(x as i32) + ey + cin as i64;
        let av = i32::try_from(exact).is_err();
        let r = if av && self.sat {
            if exact > 0 {
                0x7FFF_FFFF
            } else {
                0x8000_0000
            }
        } else {
            t as u32
        };
        self.fixed_result(n, r, t >> 32 != 0, av);
    }

    /// The fixed-point ALU opcodes of `Core::fixed_alu` that [`pe_supports`] lists.
    #[inline(always)]
    fn fixed_alu(&mut self, op: u8, n: u8, rx: u32, ry: u32) {
        let plain = |s: &mut Self, r: u32| s.fixed_result(n, r, false, false);
        match op {
            0x01 => self.fixed_addsub(n, rx, ry, 0, false),
            0x02 => self.fixed_addsub(n, rx, ry, 1, true),
            0x22 => self.fixed_addsub(n, 0, rx, 1, true),
            0x29 => self.fixed_addsub(n, rx, 1, 0, false),
            0x21 => plain(self, rx),
            0x40 => plain(self, rx & ry),
            0x41 => plain(self, rx | ry),
            0x42 => plain(self, rx ^ ry),
            0x61 => plain(self, (rx as i32).min(ry as i32) as u32),
            0x62 => plain(self, (rx as i32).max(ry as i32) as u32),
            0x63 => {
                let lim = i64::from(ry as i32).abs();
                plain(self, i64::from(rx as i32).clamp(-lim, lim) as u32);
            }
            _ => {
                // 0x0A: signed compare; CACC shifts in `Rx > Ry`, no result register
                let greater = (rx as i32) > (ry as i32);
                *self.astat = *self.astat & 0x00FF_FFFF
                    | ((*self.astat >> 24 >> 1) | u32::from(greater) << 7) << 24;
                (
                    self.f.az, self.f.an, self.f.av, self.f.ac, self.f.as_, self.f.ai,
                ) = (
                    rx == ry,
                    (rx as i32) < (ry as i32),
                    false,
                    false,
                    false,
                    false,
                );
            }
        }
    }

    /// A float ALU opcode (`Core::float_alu` + the AF update of `exec_compute_op`).
    #[inline(always)]
    fn float_alu(&mut self, op: u8, n: u8, x: u8, y: u8) {
        let o = match op {
            0x81 => self.k_add(self.f40(x), self.f40(y), false),
            0x82 => self.k_add(self.f40(x), self.f40(y), true),
            0x89 => float::add(self.f40(x), self.f40(y), false, true, self.mode),
            0x8A => {
                // compare: CACC shifts in `Fx > Fy`, no result register
                let (a, b) = (self.f40(x), self.f40(y));
                let cacc =
                    (*self.astat >> 24 >> 1) | u32::from(misc::greater(a, b, self.mode)) << 7;
                *self.astat = *self.astat & 0x00FF_FFFF | cacc << 24;
                self.apply(misc::compare(a, b, self.mode));
                (*self.astat) |= AF;
                return;
            }
            0xA1 => misc::sign_op(self.f40(x), 0, self.mode),
            0xB0 => misc::sign_op(self.f40(x), 2, self.mode),
            0xC9 | 0xD9 => {
                let iy = if op == 0xD9 {
                    self.r[usize::from(y)] as i32
                } else {
                    0
                };
                let o = conv::to_int(self.f40(x), iy, false, self.mode);
                let n = usize::from(n);
                self.r[n] = o.v as u32;
                self.lo[n] = 0;
                self.apply(o);
                (*self.astat) |= AF;
                return;
            }
            0xC5 => misc::rsqrts(self.f40(x), self.mode),
            0xDA => self.k_from_int(self.r[usize::from(x)] as i32, self.r[usize::from(y)] as i32),
            0xE0..=0xE3 => misc::select_op(self.f40(x), self.f40(y), op & 3, self.mode),
            0xA2 => misc::sign_op(self.f40(x), 1, self.mode),
            _ => misc::recips(self.f40(x), self.mode),
        };
        self.float_result(n, o);
        (*self.astat) |= AF;
    }

    /// Executes one lowered compute field on this element ([`pe_supports`] gates the ops).
    #[inline(always)]
    pub(super) fn exec(&mut self, op: ComputeOp) {
        match op {
            ComputeOp::Nop => {}
            ComputeOp::FloatAlu { op, n, x, y } => self.float_alu(op, n, x, y),
            ComputeOp::FloatMul { n, x, y } => {
                let o = self.k_mul(self.f40(x), self.f40(y));
                self.mul_result(n, o);
            }
            ComputeOp::FixedAlu { op, n, x, y } => {
                let (rx, ry) = (self.r[usize::from(x)], self.r[usize::from(y)]);
                self.fixed_alu(op, n, rx, ry);
                (*self.astat) &= !AF;
            }
            ComputeOp::Shifter { op, n, x, y } => {
                let ry = self.r[usize::from(y)];
                self.shift(u64::from(op), n, x, ry);
            }
            ComputeOp::Multifunction { c } => {
                if let Some((rm, xm, ym, alu, ra, xa, ya)) = float_multi(c) {
                    self.multi_float(rm, xm, ym, alu, ra, xa, ya);
                }
            }
            _ => {}
        }
    }

    /// `Rm = Fxm * Fym` and `Ra = <float ALU> Fxa, Fya` from the same original registers
    /// (`Core::parallel`): the ALU half runs first and its flags win.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn multi_float(&mut self, rm: u8, xm: u8, ym: u8, alu: u8, ra: u8, xa: u8, ya: u8) {
        let i = usize::from(ra);
        let orig = (self.r[i], self.tag[i], self.lo[i]);
        let orig_flags = (
            self.f.az, self.f.an, self.f.av, self.f.ac, self.f.as_, self.f.ai,
        );
        self.float_alu(alu, ra, xa, ya);
        let done = (self.r[i], self.tag[i], self.lo[i]);
        let done_flags = (
            self.f.az, self.f.an, self.f.av, self.f.ac, self.f.as_, self.f.ai,
        );
        (self.r[i], self.tag[i], self.lo[i]) = orig;
        (
            self.f.az, self.f.an, self.f.av, self.f.ac, self.f.as_, self.f.ai,
        ) = orig_flags;
        let o = self.k_mul(self.f40(xm), self.f40(ym));
        self.mul_result(rm, o);
        (self.r[i], self.tag[i], self.lo[i]) = done;
        (
            self.f.az, self.f.an, self.f.av, self.f.ac, self.f.as_, self.f.ai,
        ) = done_flags;
    }

    /// `Rn = Rimm` for a data register (`Core::write_ureg`: the float extension is dropped).
    #[inline(always)]
    pub(super) fn load_imm(&mut self, reg: u8, value: u32) {
        let n = usize::from(reg);
        self.r[n] = value;
        self.lo[n] = 0;
    }

    /// Source of `Rdst = Rsrc`: value and float extension.
    #[inline(always)]
    pub(super) fn move_src(&self, src: u8) -> (u32, u32, u8) {
        let n = usize::from(src);
        (self.r[n], self.tag[n], self.lo[n])
    }

    /// Destination of a data-register move (the extension travels with the value).
    #[inline(always)]
    pub(super) fn move_dst(&mut self, dst: u8, m: (u32, u32, u8)) {
        let n = usize::from(dst);
        (self.r[n], self.tag[n], self.lo[n]) = m;
    }

    /// `Rn = BSET|BCLR|BTGL Rx BY ry` (register-form opcode `0xC0`, `0xC4`, `0xC8`).
    #[inline(always)]
    pub(super) fn shift(&mut self, op: u64, n: u8, x: u8, ry: u32) {
        let rx = self.r[usize::from(x)];
        let old = self.r[usize::from(n)];
        let (len, pos) = ((ry >> 6) & 63, ry & 63);
        let bit = ry as i32;
        let bit_ok = (0..32).contains(&bit);
        let r = match op {
            0x00 | 0x20 | 0x04 | 0x24 => {
                let arith = op & 4 != 0;
                let r = shifter_bits::shift(rx, bit, arith) | if op & 0x20 != 0 { old } else { 0 };
                let sv = bit > 0
                    && (!arith
                        || shifter_bits::shift(rx, bit, true) as i32 >> ry.min(31) != rx as i32
                        || (ry >= 32 && (rx != 0 || op & 0x20 != 0)));
                (self.f.sz, self.f.sv, self.f.ss) = (r == 0, sv, false);
                r
            }
            0x08 => {
                let r = rx.rotate_left(bit.rem_euclid(32) as u32);
                (self.f.sz, self.f.sv, self.f.ss) = (r == 0, false, false);
                r
            }
            0xC0 | 0xC4 | 0xC8 => {
                let r = match (bit_ok, op) {
                    (false, _) => rx,
                    (true, 0xC0) => rx | (1 << bit),
                    (true, 0xC4) => rx & !(1 << bit),
                    (true, _) => rx ^ (1 << bit),
                };
                (self.f.sz, self.f.sv, self.f.ss) = (r == 0, !bit_ok, false);
                r
            }
            0xCC => {
                let zero = !bit_ok || (rx >> bit) & 1 == 0;
                (self.f.sz, self.f.sv, self.f.ss) = (zero, !bit_ok, false);
                (*self.astat) &= !SHIFTER_CLEAR;
                return;
            }
            0x44 | 0x4C | 0x64 | 0x6C => {
                let (v, over) = shifter_bits::deposit(rx, len, pos, op & 8 != 0);
                let r = v | if op & 0x20 != 0 { old } else { 0 };
                (self.f.sz, self.f.sv, self.f.ss) = (r == 0, over, false);
                r
            }
            _ => {
                let (r, over) = shifter_bits::extract(rx, len, pos, op & 8 != 0);
                (self.f.sz, self.f.sv, self.f.ss) = (r == 0, over, false);
                r
            }
        };
        self.r[usize::from(n)] = r;
        (*self.astat) &= !SHIFTER_CLEAR;
    }
}

impl Core {
    /// Records the block's accumulated arithmetic-exception bits of element `y`: STKY and the
    /// latched `FIXI`/`FLTOI`/`FLTUI`/`FLTII` interrupts, like `note_arith_exception` called
    /// from PEy's swapped view (the writable-bit mask is PEx's).
    #[cold]
    #[inline(never)]
    pub(super) fn aot_note_sticky(&mut self, y: bool, sticky: u32) {
        self.note_arith_exception_in(usize::from(y), 0, sticky);
    }

    /// Ends an in-place [`Pe`] run: raises its accumulated arithmetic-exception bits.
    #[inline(always)]
    pub(super) fn aot_pe_end(&mut self, y: bool, sticky: u32) {
        if sticky != 0 {
            self.aot_note_sticky(y, sticky);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::aot_lower::LoweredOp;
    use super::super::support::core_with;
    use super::*;

    fn run_pe(core: &mut Core, ops: &[LoweredOp], kern: Kern) {
        for y in [false, true] {
            let mut pe = Pe::load(core, y).with_kern(kern);
            for &op in ops {
                match op {
                    LoweredOp::Compute(c) => pe.exec(c),
                    LoweredOp::LoadImm { reg, value } => pe.load_imm(reg, value),
                    LoweredOp::RegMove { src, dst, compute } => {
                        let m = pe.move_src(src);
                        if let Some(c) = compute {
                            pe.exec(c);
                        }
                        pe.move_dst(dst, m);
                    }
                    LoweredOp::Shift { op, n, x, ry } => pe.shift(op, n, x, ry),
                }
            }
            let st = pe.sticky();
            core.aot_pe_end(y, st);
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn reg(&mut self) -> u8 {
            (self.next() % 6) as u8 + 8 // few registers, in the multifunction x/y ranges, to force hazards
        }
        fn float(&mut self) -> u32 {
            let v = self.next();
            match v % 12 {
                0 => 0,
                1 => 0x7F80_0000 | (v >> 40) as u32 & 0x8000_0000,
                2 => (v >> 8) as u32 & 0x807F_FFFF,
                3 => 0x7FFF_FFFF,
                4 => (v >> 8) as u32,
                _ => ((v >> 8) as u32 & 0x807F_FFFF) | ((120 + (v >> 50) as u32 % 14) << 23),
            }
        }
    }

    /// A copy of the state the ops touch (registers, floats extensions, flags, ASTAT, STKY, mode).
    fn clone_state(c: &Core) -> Core {
        let mut n = core_with(&[0x0001]);
        n.mode1 = c.mode1;
        n.r = c.r;
        n.s = c.s;
        n.flags = c.flags;
        n.flags_y = c.flags_y;
        n.sys.astat_extra = c.sys.astat_extra;
        n.sys.stky = c.sys.stky;
        n.fx = c.fx;
        n.fx_y = c.fx_y;
        n
    }

    fn random_op(r: &mut Rng) -> LoweredOp {
        let (n, x, y) = (r.reg() % 8, r.reg(), r.reg() + 4);
        let (n, x, y) = (n, x.min(15), y.min(15));
        let falu = |op| ComputeOp::FloatAlu { op, n, x, y };
        let ops = [
            0x81, 0x82, 0xDA, 0xE0, 0xE1, 0xE2, 0xE3, 0xA2, 0xC4, 0x89, 0x8A, 0xA1, 0xB0, 0xC5,
            0xC9, 0xD9,
        ];
        match r.next() % 14 {
            12 | 13 => LoweredOp::Compute(ComputeOp::Shifter {
                op: [
                    0x00, 0x04, 0x08, 0x20, 0x24, 0x40, 0x44, 0x48, 0x4C, 0x64, 0x6C, 0xC0, 0xC4,
                    0xC8, 0xCC,
                ][(r.next() % 15) as usize],
                n,
                x,
                y,
            }),
            0..=3 => LoweredOp::Compute(falu(ops[(r.next() % 16) as usize])),
            4 | 5 => LoweredOp::Compute(ComputeOp::FloatMul { n, x, y }),
            6 => LoweredOp::Compute(ComputeOp::FixedAlu {
                op: [
                    0x01, 0x02, 0x0A, 0x21, 0x22, 0x29, 0x40, 0x41, 0x42, 0x61, 0x62, 0x63,
                ][(r.next() % 12) as usize],
                n,
                x,
                y,
            }),
            7 => {
                let c = (1u64 << 22)
                    | (3 << 19)
                    | ((r.next() % 8) << 16)
                    | (u64::from(n) << 12)
                    | (u64::from(n.wrapping_add(1) & 15) << 8)
                    | ((r.next() % 4) << 6)
                    | ((r.next() % 4) << 4)
                    | ((r.next() % 4) << 2)
                    | (r.next() % 4);
                LoweredOp::Compute(ComputeOp::Multifunction { c })
            }
            8 => LoweredOp::LoadImm {
                reg: n,
                value: r.float(),
            },
            9 => LoweredOp::RegMove {
                src: x,
                dst: n,
                compute: (r.next() & 1 == 0).then(|| falu(0x82)),
            },
            _ => LoweredOp::Shift {
                op: [
                    0xC0, 0xC4, 0xC8, 0x00, 0x04, 0x08, 0x20, 0x24, 0x40, 0x44, 0x48, 0x4C, 0x64,
                    0x6C, 0xCC,
                ][(r.next() % 15) as usize],
                n,
                x,
                ry: if r.next() & 1 == 0 {
                    (r.next() % 40) as u32
                } else {
                    r.next() as u32 >> (r.next() % 20)
                },
            },
        }
    }

    #[test]
    fn pe_ops_equal_interpreter_ops_on_random_simd_state() {
        check_against_interpreter(Kern::Exact);
    }

    #[test]
    fn guarded_pe_ops_equal_interpreter_ops_on_random_simd_state() {
        check_against_interpreter(Kern::Guarded);
    }

    fn check_against_interpreter(kern: Kern) {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        let mut tested = 0;
        for case in 0..6000 {
            let ops: Vec<LoweredOp> = (0..12)
                .map(|_| random_op(&mut r))
                .filter(|op| match op {
                    LoweredOp::Compute(c) => pe_supports(*c),
                    LoweredOp::RegMove { compute, .. } => compute.is_none_or(pe_supports),
                    LoweredOp::Shift { op, .. } => pe_supports_shift(*op),
                    LoweredOp::LoadImm { .. } => true,
                })
                .collect();
            let mut a = core_with(&[0x0001]);
            let mut b = core_with(&[0x0001]);
            for c in [&mut a, &mut b] {
                c.mode1 = (1 << 21) | [0, 1 << 15, 1 << 16, 1 << 13, 1 << 31][case % 5];
            }
            for k in 0..16 {
                let (x, y) = (r.float(), r.float());
                (a.r[k], b.r[k]) = (x, x);
                (a.s[k], b.s[k]) = (y, y);
            }
            let flags = r.next();
            for c in [&mut a, &mut b] {
                c.flags.ac = flags & 1 == 1;
                c.flags_y.az = flags & 2 != 0;
                c.sys.astat_extra = [(flags >> 8) as u32 & 0x4FF, (flags >> 24) as u32 & 0x4FF];
            }
            let (a0, b0) = (clone_state(&a), clone_state(&b));
            a.run_lowered(&ops);
            run_pe(&mut b, &ops, kern);
            if let Some(m) = a.state_mismatch(&b) {
                // report the shortest failing prefix of the sequence
                for k in 1..=ops.len() {
                    let (mut pa, mut pb) = (clone_state(&a0), clone_state(&b0));
                    pa.run_lowered(&ops[..k]);
                    run_pe(&mut pb, &ops[..k], kern);
                    if let Some(mk) = pa.state_mismatch(&pb) {
                        panic!(
                            "case {case}: first failing op {:?} (after {k}): {mk}",
                            ops[k - 1]
                        );
                    }
                }
                panic!("case {case}: {m}");
            }
            tested += ops.len();
        }
        assert!(tested > 40_000, "{tested}");
    }

    #[test]
    fn float_multi_decodes_only_the_float_with_float_alu_forms() {
        // the ORG word `F6 = F0 * F6, F0 = F11 - F14`
        let (rm, xm, ym, alu, ra, xa, ya) = float_multi(5_857_326).unwrap();
        assert_eq!((rm, alu, ra), (6, 0x82, 0));
        assert_eq!((xm, ym, xa, ya), (0, 6, 11, 14));
        assert!(float_multi(1 << 21).is_none(), "three-operation form");
        assert!(float_multi(1 << 22).is_none(), "register transfer");
        assert!(!pe_supports(ComputeOp::Shifter {
            op: 0x80,
            n: 0,
            x: 0,
            y: 0
        }));
        assert!(pe_supports_shift(0xC8) && pe_supports_shift(0x00) && !pe_supports_shift(0x80));
    }
}
