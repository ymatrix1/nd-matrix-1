//! Exact float kernels: pure functions on 40-bit register values, the flag outputs and the MODE1
//! bits, bit-identical to the interpreter's `float::*` (design 4.4).
//!
//! OWNER: W1-KERN (this file and its siblings). Handlers (`exec/h_float.rs`) and the native
//! emitters call these, never `float::*` directly, so a faster implementation replaces the body
//! of one kernel with the exact grid test of that file as its check.
//!
//! Where each kernel comes from (reuse, not a rewrite: #685 is merged and `pub(super)` of `core`):
//!
//! | Kernel | Body |
//! |---|---|
//! | `fadd`, `fsub`, `fmul`, `ffloat` | `aot_fast::{fast_add, fast_mul, fast_from_int}` (scalar, f64 add trick plus `aot_round::round_norm`) |
//! | `fadd2`, `fsub2`, `fmul2` | `aot_simd::{fast_add2, fast_sub2, fast_mul2}` (PEx and PEy as one SSE2 pair) |
//! | `favg`, `favg2` | the add kernel, then one exponent decrement (`avg.rs`: the old u128 path is gone) |
//! | `fsign`, `fselect`, `fcmp`, `fgreater` | integer compare of the flushed magnitudes (`operand.rs`) |
//! | `ffix`, `frnd`, `fscalb` | `u64` arithmetic and `round_norm` instead of `u128` (`convert.rs`) |
//!
//! Every kernel takes the [`Mode`] explicitly (the contract signature `fadd(a, b, st)` lacks it;
//! rounding and saturation need MODE1, so it is an input here). The `*2` forms take the PEx
//! operand in lane 0 and the PEy operand in lane 1 and equal two scalar calls.
//!
//! Provenance: VERIFIED bit-identical (value, every flag of [`FlagsOut`]) to the `float::*`,
//! `float_misc::*` and `float_conv::*` reference by the class grids and seeded streams in the
//! `tests` module of each kernel file (
//! `add_and_sub_equal_reference_on_the_class_grid_and_seeded_streams`,
//! `mul_equals_reference_on_the_class_grid_and_seeded_streams`, the `avg`, `select`, `compare`
//! and `convert` tests of the same shape, and the paired forms of add, subtract, multiply and
//! average), and VERIFIED against the interpreter and `Pe::exec` including the hidden
//! extension tag and low byte by `exec/pe/diff.rs`. The reference kernels are VERIFIED against
//! the CCES simulator (`float.rs`). Input values are clean 40-bit values (`Core::f40`).
pub mod add;
pub mod avg;
pub mod compare;
pub mod convert;
#[cfg(test)]
pub(crate) mod grid;
pub mod mul;
mod operand;
#[cfg(test)]
mod probe;
pub mod select;

pub use crate::core::float::Mode;
use crate::core::float::Out;

/// A 40-bit register float (or a 32-bit integer in the low bits), as `Core` keeps it.
pub type F40 = u64;

/// The ASTAT-side outputs of one float operation (what `float_ops` applies after it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlagsOut {
    pub az: bool,
    pub an: bool,
    pub av: bool,
    pub as_: bool,
    pub ai: bool,
    pub mn: bool,
    pub mv: bool,
    pub mu: bool,
    pub mi: bool,
    pub au: bool,
}

impl FlagsOut {
    /// The flags of an interpreter result.
    pub fn of(o: &Out) -> Self {
        FlagsOut {
            az: o.az,
            an: o.an,
            av: o.av,
            as_: o.as_,
            ai: o.ai,
            mn: o.mn,
            mv: o.mv,
            mu: o.mu,
            mi: o.mi,
            au: o.au,
        }
    }

    /// The interpreter result with value `v` and these flags: the inverse of [`FlagsOut::of`],
    /// used to compare a kernel with the reference through `Out`'s `PartialEq`.
    pub fn out(self, v: F40) -> Out {
        Out {
            v,
            az: self.az,
            an: self.an,
            av: self.av,
            as_: self.as_,
            ai: self.ai,
            mn: self.mn,
            mv: self.mv,
            mu: self.mu,
            mi: self.mi,
            au: self.au,
        }
    }
}

/// Stores the flags of `o` into `st` and returns its value. The one place a kernel result is
/// split, so every kernel reports flags the same way.
#[inline(always)]
pub(crate) fn finish(o: Out, st: &mut FlagsOut) -> F40 {
    *st = FlagsOut::of(&o);
    o.v
}

/// [`finish`] for the two lanes of a paired kernel.
#[inline(always)]
pub(crate) fn finish2(o: [Out; 2], st: &mut [FlagsOut; 2]) -> [F40; 2] {
    [finish(o[0], &mut st[0]), finish(o[1], &mut st[1])]
}

/// The register image of an `f32` (top 32 bits, empty extension), for tests.
#[cfg(test)]
pub(crate) fn f40(x: f32) -> F40 {
    u64::from(x.to_bits()) << 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_out_copies_every_flag_of_the_interpreter_result() {
        let o = Out {
            v: 7,
            az: true,
            an: false,
            av: true,
            as_: false,
            ai: true,
            mn: false,
            mv: true,
            mu: false,
            mi: true,
            au: true,
        };
        let mut st = FlagsOut::default();
        assert_eq!(finish(o, &mut st), 7);
        assert_eq!(
            st,
            FlagsOut {
                az: true,
                an: false,
                av: true,
                as_: false,
                ai: true,
                mn: false,
                mv: true,
                mu: false,
                mi: true,
                au: true
            }
        );
        assert_eq!(f40(1.0), 0x3F_8000_0000);
    }

    #[test]
    fn out_is_the_inverse_of_of_and_finish2_fills_both_lanes() {
        let o = Out {
            v: 0x12_3456_789A,
            az: true,
            an: true,
            av: false,
            as_: true,
            ai: false,
            mn: true,
            mv: false,
            mu: true,
            mi: false,
            au: true,
        };
        assert_eq!(FlagsOut::of(&o).out(o.v), o);
        let p = Out::invalid();
        let mut st = [FlagsOut::default(); 2];
        assert_eq!(finish2([o, p], &mut st), [o.v, p.v]);
        assert_eq!(st, [FlagsOut::of(&o), FlagsOut::of(&p)]);
        assert_ne!(st[0], st[1], "the lanes are independent");
    }
}
