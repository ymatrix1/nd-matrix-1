//! One processing element (PEx or PEy) borrowed in place for the compute handlers: the data
//! registers, the float extensions, the ALU flags and the ASTAT extra bits stay in the [`Core`]
//! and are read and written through this view, so one compute op does not copy the element
//! state in and out. The arithmetic-exception bits an op raises are collected in `sticky` and
//! handed to the core once, after the borrow has ended (`Core::aot_pe_end`, like `Pe`).
//!
//! OWNER: W1-KERN. This is the element view the diet kernels of `kernels/` need: it reads an
//! operand as a 40-bit float or a 32-bit integer, writes a float or an integer result with the
//! extension rules of `float_ops.rs`, and applies a [`FlagsOut`] to the flags, ASTAT and STKY the
//! way `Core::float_result` / `Core::mul_result` do. [`elems`] borrows both elements at once (the
//! two register files are disjoint fields of the core), which the paired kernels need.
//!
//! Provenance: VERIFIED equal to `Pe::exec` and to the interpreter (`Core::exec_compute_op` on
//! PEx, then on the swapped view for PEy) in every written bit, including the hidden extension
//! tag and low byte, by `kernel_ops_equal_interpreter_and_pe_exec` in `diff.rs`.
use crate::core::float::Mode;
use crate::core::types::Flags;
use crate::core::uop::kernels::{FlagsOut, F40};
use crate::core::Core;

/// ASTAT extra bit 10 (AF): the last ALU operation was floating-point.
/// VERIFIED: `alu.rs` (`exec_compute_op` sets it for every float ALU op and leaves the
/// multiplier alone), checked by `kernel_ops_equal_interpreter_and_pe_exec`.
pub(in crate::core) const AF: u32 = 1 << 10;
/// Position and width of the multiplier flags MN, MV, MU, MI in the ASTAT extra bits.
/// VERIFIED: `float_ops.rs::mul_result`, checked by `kernel_ops_equal_interpreter_and_pe_exec`.
const MULT_SHIFT: u32 = 6;

/// A processing element borrowed from the core for one handler.
pub(in crate::core) struct Elem<'a> {
    r: &'a mut [u32; 16],
    tag: &'a mut [u32; 16],
    lo: &'a mut [u8; 16],
    f: &'a mut Flags,
    astat: &'a mut u32,
    sticky: u32,
    mode: Mode,
}

/// Borrows PEx and PEy of `core` together, both with the float mode of MODE1.
#[inline(always)]
pub(in crate::core) fn elems(core: &mut Core) -> (Elem<'_>, Elem<'_>) {
    let mode = Mode::from_mode1(core.mode1);
    let [ax, ay] = &mut core.sys.astat_extra;
    (
        Elem {
            r: &mut core.r,
            tag: &mut core.fx.tag,
            lo: &mut core.fx.lo,
            f: &mut core.flags,
            astat: ax,
            sticky: 0,
            mode,
        },
        Elem {
            r: &mut core.s,
            tag: &mut core.fx_y.tag,
            lo: &mut core.fx_y.lo,
            f: &mut core.flags_y,
            astat: ay,
            sticky: 0,
            mode,
        },
    )
}

impl Elem<'_> {
    /// The float rounding and saturation mode of MODE1 at borrow time.
    #[inline(always)]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The 40-bit float in register `n` (the extension counts while its tag matches).
    #[inline(always)]
    pub fn f40(&self, n: u8) -> F40 {
        let n = usize::from(n & 15);
        let lo = if self.tag[n] == self.r[n] {
            self.lo[n]
        } else {
            0
        };
        u64::from(self.r[n]) << 8 | u64::from(lo)
    }

    /// The 32-bit integer in register `n`.
    #[inline(always)]
    pub fn int(&self, n: u8) -> i32 {
        self.r[usize::from(n & 15)] as i32
    }

    /// `Fn = v`: the register, and the extension valid for it (`Core::set_f40`).
    #[inline(always)]
    pub fn set_f40(&mut self, n: u8, v: F40) {
        let n = usize::from(n & 15);
        self.r[n] = (v >> 8) as u32;
        self.tag[n] = self.r[n];
        self.lo[n] = v as u8;
    }

    /// `Rn = v` for an integer result: the extension is dropped, its tag is left alone
    /// (`Core::int_result`: a later float read sees a stale extension as zero).
    #[inline(always)]
    pub fn set_int(&mut self, n: u8, v: u32) {
        let n = usize::from(n & 15);
        self.r[n] = v;
        self.lo[n] = 0;
    }

    /// The ALU flags of a float result, the arithmetic-exception bits they latch (AUS, AVS, AIS)
    /// and AF (`Core::apply` and the AF update of `exec_compute_op`). AC is cleared.
    #[inline(always)]
    pub fn alu_flags(&mut self, st: &FlagsOut) {
        let f = &mut *self.f;
        (f.az, f.an, f.av, f.ac, f.as_, f.ai) = (st.az, st.an, st.av, false, st.as_, st.ai);
        self.sticky |= u32::from(st.au) | u32::from(st.av) << 1 | u32::from(st.ai) << 5;
        *self.astat |= AF;
    }

    /// The multiplier flags MN, MV, MU, MI (ASTAT bits 6..9) and the sticky MVS, MUS, MIS
    /// (`Core::mul_result`). The ALU flags and AF stay.
    #[inline(always)]
    pub fn mul_flags(&mut self, st: &FlagsOut) {
        let mult = u32::from(st.mn)
            | u32::from(st.mv) << 1
            | u32::from(st.mu) << 2
            | u32::from(st.mi) << 3;
        *self.astat = *self.astat & !(0xF << MULT_SHIFT) | mult << MULT_SHIFT;
        self.sticky |= u32::from(st.mv) << 7 | u32::from(st.mu) << 8 | u32::from(st.mi) << 9;
    }

    /// Shifts `Rx > Ry` into the compare accumulator (`Core::shift_cacc`).
    #[inline(always)]
    pub fn shift_cacc(&mut self, greater: bool) {
        let cacc = (*self.astat >> 24 >> 1) | u32::from(greater) << 7;
        *self.astat = *self.astat & 0x00FF_FFFF | cacc << 24;
    }

    /// The arithmetic-exception bits collected so far; pass them to `Core::aot_pe_end` once
    /// the borrow has ended.
    #[inline(always)]
    pub fn sticky(&self) -> u32 {
        self.sticky
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    fn core() -> Core {
        core_with(&[0x0001])
    }

    #[test]
    fn the_two_views_borrow_the_two_register_files_and_flag_sets() {
        let mut c = core();
        c.r[3] = 0x1111;
        c.s[3] = 0x2222;
        c.mode1 = 1 << 15 | 1 << 16;
        let (mut x, mut y) = elems(&mut c);
        assert_eq!((x.int(3), y.int(3)), (0x1111, 0x2222));
        assert!(x.mode().trunc && y.mode().r32);
        x.set_f40(4, 0x12_3456_78AB);
        y.set_int(4, 7);
        x.alu_flags(&FlagsOut {
            az: true,
            ..FlagsOut::default()
        });
        y.shift_cacc(true);
        let (sx, sy) = (x.sticky(), y.sticky());
        assert_eq!(
            (c.r[4], c.fx.tag[4], c.fx.lo[4]),
            (0x1234_5678, 0x1234_5678, 0xAB)
        );
        assert_eq!((c.s[4], c.fx_y.lo[4]), (7, 0));
        assert!(c.flags.az && !c.flags_y.az);
        assert_eq!(c.sys.astat_extra, [AF, 0x80 << 24]);
        assert_eq!((sx, sy), (0, 0));
    }

    #[test]
    fn a_float_read_ignores_a_stale_extension_and_an_integer_write_leaves_the_tag() {
        let mut c = core();
        c.r[1] = 0x4000_0000;
        c.fx.tag[1] = 0x4000_0000;
        c.fx.lo[1] = 0x5A;
        {
            let (mut x, _) = elems(&mut c);
            assert_eq!(x.f40(1), 0x40_0000_005A);
            x.set_int(1, 0x4000_0001);
            assert_eq!(
                x.f40(1),
                0x40_0000_0100,
                "the tag no longer matches: extension reads 0"
            );
            x.set_int(1, 0x4000_0000);
            assert_eq!(
                x.f40(1),
                0x40_0000_0000,
                "the extension was cleared, not just hidden"
            );
        }
        assert_eq!((c.fx.tag[1], c.fx.lo[1]), (0x4000_0000, 0));
    }

    #[test]
    fn flag_application_matches_the_documented_bit_positions() {
        let mut c = core();
        c.sys.astat_extra = [0xFFFF_FFFF, 0];
        c.flags.ac = true;
        let (mut x, _) = elems(&mut c);
        x.alu_flags(&FlagsOut {
            av: true,
            ai: true,
            au: true,
            ..FlagsOut::default()
        });
        assert_eq!(x.sticky(), 1 | 1 << 1 | 1 << 5);
        x.mul_flags(&FlagsOut {
            mn: true,
            mu: true,
            ..FlagsOut::default()
        });
        assert_eq!(x.sticky(), 1 | 1 << 1 | 1 << 5 | 1 << 8);
        x.shift_cacc(false);
        assert!(!c.flags.ac, "an ALU result clears AC");
        assert!(c.flags.av && c.flags.ai);
        assert_eq!(
            c.sys.astat_extra[0],
            0x7FFF_FFFF & !(0xF << 6) | (1 | 1 << 2) << 6
        );
    }
}
