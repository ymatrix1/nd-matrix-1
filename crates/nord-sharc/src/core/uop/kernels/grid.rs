//! Operand grids and seeded streams shared by the kernel and the element tests.
//!
//! OWNER: W1-KERN, test support only. [`grid`] is the class grid: every operand class (zero,
//! negative zero, denormal, normal, infinity, NaN), both signs, the exponent edges (1, 2, 126 to
//! 128, 253, 254) and mantissas that sit on a rounding boundary (empty, last bit of the 40-bit
//! and the 32-bit significand, half of the extension, all ones), with and without extension bits.
//! [`operand`] draws the same classes at random, biased to the sticky cases of the add kernel
//! (equal exponents, near cancellation, large gaps). A [`Seen`] tally lets a test assert that its
//! loop reached every class and every flag, so a differential test cannot pass vacuously.
use super::{FlagsOut, Mode, F40};
use crate::core::float::{self, Class as RefClass};
use crate::core::uop::verify::gen::Rng;

/// Operand class as the reference decoder sees it, with the sign of a zero kept apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    PosZero,
    NegZero,
    Denormal,
    Normal,
    Inf,
    Nan,
}

/// Class of `v`, decided by `float::unpack` (not by the kernels under test).
pub(crate) fn class_of(v: F40) -> Class {
    let u = float::unpack(v, false);
    match u.class {
        RefClass::Zero if v & 0x7FFF_FFFF != 0 => Class::Denormal,
        RefClass::Zero if u.neg => Class::NegZero,
        RefClass::Zero => Class::PosZero,
        RefClass::Norm => Class::Normal,
        RefClass::Inf => Class::Inf,
        RefClass::Nan => Class::Nan,
    }
}

/// The class grid, 144 operands: every sign, every exponent edge and every mantissa pattern.
pub(crate) fn grid() -> Vec<F40> {
    const EXPS: [u64; 9] = [0, 1, 2, 126, 127, 128, 253, 254, 255];
    // empty, last bit of the 32-bit significand, half of the extension, extension only, a low
    // run, the top mantissa bit, all ones, the pattern of a tie at 24 bits
    const FRACS: [u64; 8] = [
        0,
        0x100,
        0x80,
        0xFF,
        0xFFFF,
        0x4000_0000,
        0x7FFF_FFFF,
        0x0000_7F80,
    ];
    let mut v = Vec::new();
    for sign in [0u64, 1] {
        for e in EXPS {
            for f in FRACS {
                v.push(sign << 39 | e << 31 | f);
            }
        }
    }
    v
}

/// The eight MODE1 float settings: TRUNC x RND32 x ALUSAT.
pub(crate) fn modes() -> [Mode; 8] {
    std::array::from_fn(|k| Mode {
        trunc: k & 1 != 0,
        r32: k & 2 != 0,
        sat: k & 4 != 0,
    })
}

/// A 40-bit float drawn from an operand class: ordinary, near one, huge and tiny exponents, exact
/// powers of two, specials (zero, denormal, infinity, NaN) and values with a set extension.
pub(crate) fn operand(r: &mut Rng) -> F40 {
    let v = r.next_u64();
    let (sign, frac, ext) = ((v >> 63) << 39, (v >> 8) & 0x7FFF_FFFF, v & 0xFF);
    let exp = match (v >> 40) % 16 {
        0 => 0,
        1 => 255,
        2 => 1 + r.next_u64() % 2,
        3 => 253 + r.next_u64() % 2,
        4 | 5 => 127 + r.next_u64() % 5 - 2,
        6 => 100 + r.next_u64() % 55,
        _ => 1 + r.next_u64() % 254,
    };
    // an empty mantissa is often an exact power of two or an exact zero (empty extension too)
    let (frac, ext) = match (v >> 44) % 8 {
        0 => (0, if v & 0x100 != 0 { 0 } else { ext }),
        1 => (0x7FFF_FFFF, ext),
        _ => (frac, ext),
    };
    sign | exp << 31 | frac | ext
}

/// An operand near `a`: the same exponent shifted by a small gap and a mantissa that is close to
/// it or to its complement, so sums cancel and round on a boundary.
pub(crate) fn near(r: &mut Rng, a: F40) -> F40 {
    let gap = r.next_u64() % 40;
    let exp = ((a >> 31) & 0xFF).wrapping_add(20).wrapping_sub(gap) & 0xFF;
    let flip = r.next_u64() & 0x3FF;
    let sign = if r.next_u64() & 1 == 0 {
        a & (1 << 39)
    } else {
        !a & (1 << 39)
    };
    sign | exp << 31 | ((a ^ flip) & 0x7FFF_FFFF)
}

/// A flag word with every flag set: a kernel must overwrite all of it, never leave one alone.
pub(crate) fn dirty() -> FlagsOut {
    FlagsOut {
        az: true,
        an: true,
        av: true,
        as_: true,
        ai: true,
        mn: true,
        mv: true,
        mu: true,
        mi: true,
        au: true,
    }
}

/// What a test loop reached: operand classes and every flag of the results it compared.
#[derive(Debug, Default)]
pub(crate) struct Seen {
    pub classes: [usize; 6],
    pub flags: [usize; 10],
    pub cases: usize,
}

impl Seen {
    /// Counts the class of an operand.
    pub fn operand(&mut self, v: F40) {
        self.classes[class_of(v) as usize] += 1;
    }

    /// Counts a compared result and which of its flags were set.
    pub fn result(&mut self, st: &FlagsOut) {
        let f = [
            st.az, st.an, st.av, st.as_, st.ai, st.mn, st.mv, st.mu, st.mi, st.au,
        ];
        for (n, set) in self.flags.iter_mut().zip(f) {
            *n += usize::from(set);
        }
        self.cases += 1;
    }

    /// Asserts the loop compared at least `min_cases` results, reached every operand class and
    /// set every flag listed in `flags` (indexes of [`FlagsOut`] in field order).
    pub fn assert_reached(&self, min_cases: usize, flags: &[usize]) {
        assert!(
            self.cases >= min_cases,
            "only {} cases: {self:?}",
            self.cases
        );
        assert!(
            self.classes.iter().all(|&n| n > 0),
            "operand classes: {self:?}"
        );
        for &f in flags {
            assert!(self.flags[f] > 0, "flag {f} never set: {self:?}");
        }
    }
}

/// Field indexes of [`FlagsOut`] in [`Seen::flags`].
pub(crate) const AZ: usize = 0;
pub(crate) const AN: usize = 1;
pub(crate) const AV: usize = 2;
pub(crate) const AS: usize = 3;
pub(crate) const AI: usize = 4;
pub(crate) const MN: usize = 5;
pub(crate) const MV: usize = 6;
pub(crate) const MU: usize = 7;
pub(crate) const MI: usize = 8;
pub(crate) const AU: usize = 9;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_covers_every_class_edge_and_a_seeded_stream_is_reproducible() {
        let g = grid();
        assert_eq!(g.len(), 144);
        let mut seen = Seen::default();
        for &v in &g {
            seen.operand(v);
        }
        assert!(seen.classes.iter().all(|&n| n > 0), "{seen:?}");
        // the edges the add, avg and convert kernels branch on
        for e in [1u64, 2, 253, 254] {
            assert!(g.iter().any(|&v| (v >> 31) & 0xFF == e), "exponent {e}");
        }
        assert!(g.iter().any(|&v| class_of(v) == Class::Nan && v >> 39 == 1));
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        let stream = |r: &mut Rng| (0..64).map(|_| operand(r)).collect::<Vec<_>>();
        assert_eq!(stream(&mut a), stream(&mut b));
        let mut r = Rng::new(9);
        let classes: Vec<Class> = (0..4000).map(|_| class_of(operand(&mut r))).collect();
        for c in [
            Class::PosZero,
            Class::NegZero,
            Class::Denormal,
            Class::Normal,
            Class::Inf,
            Class::Nan,
        ] {
            assert!(classes.contains(&c), "the stream never draws {c:?}");
        }
        assert_eq!(
            modes().iter().filter(|m| m.trunc && m.r32 && m.sat).count(),
            1
        );
        assert_ne!(near(&mut r, 127 << 31), 127 << 31);
        assert!(dirty().az && dirty().au && dirty().mi && dirty().as_);
    }
}
