//! Public batch entry points over the native float kernels, for the `kernel-bench` binary in
//! `nord-tools` (the kernels themselves are crate-private). Each function runs one kernel over a
//! slice of operand pairs and folds every output field into a checksum, so the optimizer cannot
//! drop the flag derivation. `Kernel::Identity` runs the same fold over the raw operands and gives
//! the loop overhead to subtract.
//!
//! Provenance: no constants; the checksum equality between `Fast` and `Reference` over the same
//! slice is the integration check (see the test).
use super::aot_fast::{fast_add, fast_mul};
use super::aot_simd::{fast_add2, fast_mul2};
use super::float::{self, Mode, Out};

/// Which implementation a batch call runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kernel {
    /// The native-width kernels of `aot_fast.rs`.
    Fast,
    /// The paired kernels of `aot_simd.rs`: two consecutive operand pairs per call (the PEx and
    /// PEy elements of a SIMD-mode instruction), same checksum as `Fast`.
    Pair,
    /// The `u128` reference kernels of `float.rs` / `float_mul.rs`.
    Reference,
    /// No arithmetic: loop and checksum overhead only.
    Identity,
}

/// Every field of `o` mixed into `acc`.
#[inline(always)]
fn fold(acc: u64, o: Out) -> u64 {
    let flags = u64::from(o.az)
        | u64::from(o.an) << 1
        | u64::from(o.av) << 2
        | u64::from(o.as_) << 3
        | u64::from(o.ai) << 4
        | u64::from(o.mn) << 5
        | u64::from(o.mv) << 6
        | u64::from(o.mu) << 7
        | u64::from(o.mi) << 9
        | u64::from(o.au) << 8;
    acc.rotate_left(7) ^ o.v ^ (flags << 40)
}

#[inline(always)]
fn identity(a: u64, b: u64) -> Out {
    // data-dependent flags, so the fold costs the same as for a real kernel
    let bit = |v: u64, n: u32| (v >> n) & 1 == 1;
    Out {
        v: a ^ b,
        az: bit(a, 1),
        an: bit(a, 39),
        av: bit(b, 2),
        as_: bit(b, 3),
        ai: bit(a, 4),
        mn: bit(b, 5),
        mv: bit(a, 6),
        mu: bit(b, 7),
        mi: bit(b, 9),
        au: bit(a, 8),
    }
}

fn mode(trunc: bool, r32: bool) -> Mode {
    Mode {
        trunc,
        r32,
        sat: false,
    }
}

/// Checksum of `Fx + Fy` (`sub`: `Fx - Fy`) over `pairs`.
pub fn fold_add(pairs: &[(u64, u64)], sub: bool, trunc: bool, r32: bool, k: Kernel) -> u64 {
    let m = mode(trunc, r32);
    let mut acc = 0;
    match k {
        Kernel::Fast => pairs
            .iter()
            .for_each(|&(a, b)| acc = fold(acc, fast_add(a, b, sub, m))),
        Kernel::Pair => pairs.chunks_exact(2).for_each(|c| {
            let o = fast_add2([c[0].0, c[1].0], [c[0].1, c[1].1], sub, m);
            acc = fold(fold(acc, o[0]), o[1]);
        }),
        Kernel::Reference => pairs
            .iter()
            .for_each(|&(a, b)| acc = fold(acc, float::add(a, b, sub, false, m))),
        Kernel::Identity => pairs
            .iter()
            .for_each(|&(a, b)| acc = fold(acc, identity(a, b))),
    }
    acc
}

/// Checksum of `Fx * Fy` over `pairs`.
pub fn fold_mul(pairs: &[(u64, u64)], trunc: bool, r32: bool, k: Kernel) -> u64 {
    let m = mode(trunc, r32);
    let mut acc = 0;
    match k {
        Kernel::Fast => pairs
            .iter()
            .for_each(|&(a, b)| acc = fold(acc, fast_mul(a, b, m))),
        Kernel::Pair => pairs.chunks_exact(2).for_each(|c| {
            let o = fast_mul2([c[0].0, c[1].0], [c[0].1, c[1].1], m);
            acc = fold(fold(acc, o[0]), o[1]);
        }),
        Kernel::Reference => pairs
            .iter()
            .for_each(|&(a, b)| acc = fold(acc, float::mul(a, b, m))),
        Kernel::Identity => pairs
            .iter()
            .for_each(|&(a, b)| acc = fold(acc, identity(a, b))),
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_and_reference_checksums_agree_and_identity_differs() {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let pairs: Vec<(u64, u64)> = (0..2000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let a = x & 0xFF_FFFF_FFFF;
                (a, (x >> 20) & 0xFF_FFFF_FFFF)
            })
            .collect();
        for (trunc, r32) in [(false, false), (true, false), (false, true)] {
            for sub in [false, true] {
                assert_eq!(
                    fold_add(&pairs, sub, trunc, r32, Kernel::Fast),
                    fold_add(&pairs, sub, trunc, r32, Kernel::Reference)
                );
                assert_eq!(
                    fold_add(&pairs, sub, trunc, r32, Kernel::Pair),
                    fold_add(&pairs, sub, trunc, r32, Kernel::Reference)
                );
            }
            assert_eq!(
                fold_mul(&pairs, trunc, r32, Kernel::Pair),
                fold_mul(&pairs, trunc, r32, Kernel::Reference)
            );
            assert_eq!(
                fold_mul(&pairs, trunc, r32, Kernel::Fast),
                fold_mul(&pairs, trunc, r32, Kernel::Reference)
            );
        }
        assert_ne!(
            fold_mul(&pairs, false, false, Kernel::Fast),
            fold_mul(&pairs, false, false, Kernel::Identity)
        );
    }
}
