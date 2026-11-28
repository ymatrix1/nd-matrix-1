//! Exact float kernels: pure functions on 40-bit register values, the flag outputs and the MODE1
//! bits, bit-identical to the interpreter's `float::*` (design 4.4).
//!
//! OWNER: W1-KERN (this file and its siblings). Handlers (`exec/h_float.rs`) and the native
//! emitters call these, never `float::*` directly, so a faster implementation (the f64 add trick
//! of the spikes, SIMD lanes) replaces the body of one kernel with the exact grid test of that
//! file as its check.
//!
//! Reuse, do not rewrite: `aot_fast::{fast_add, fast_mul, fast_from_int}` (scalar, VERIFIED
//! bit-identical to `float::*`, f64 add trick plus `aot_round::round_norm`) and
//! `aot_simd::{fast_add2, fast_sub2, fast_mul2}` (paired PEx+PEy, SSE2, VERIFIED by
//! `aot_simd_sweep.rs`) are on `main` since #685 and are `pub(super)` of `core`, so this module
//! can call them. New work here is the paired wrappers, the avg/select/compare/convert diet and
//! the instruction-count probe. `kernels/mod.rs` belongs to W1-KERN (not W0).
//!
//! Skeleton: every kernel forwards to `float::*` and copies the flags into [`FlagsOut`]. A kernel
//! takes the [`Mode`] explicitly (the contract signature `fadd(a, b, st)` lacks it; rounding and
//! saturation need MODE1, so it is an input here).
pub mod add;
pub mod avg;
pub mod compare;
pub mod convert;
pub mod mul;
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
}

/// Stores the flags of `o` into `st` and returns its value. The one place a kernel result is
/// split, so every kernel reports flags the same way.
#[inline(always)]
pub(crate) fn finish(o: Out, st: &mut FlagsOut) -> F40 {
    *st = FlagsOut::of(&o);
    o.v
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
}
