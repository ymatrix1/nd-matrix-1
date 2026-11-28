//! `(Fx + Fy) / 2`: the exponent is decremented before rounding, so the result differs from
//! add-then-halve at the edges of the range (the 0x89 reference instruction, design 4.4).
//!
//! OWNER: W1-KERN. The reference (`float::add` with `halve`) goes through the `u128` path. The
//! diet runs the scalar add kernel and corrects its exponent:
//!
//! Halving is an exact scaling, so the rounded significand of the average is the one of the sum
//! (round to nearest even and truncation both look at the discarded bits only) and only the
//! exponent differs by one; the range checks move with it. For a sum `o = fast_add(a, b)`:
//!
//! | `o` | average |
//! |---|---|
//! | normal, exponent field 2..=254 | `o` with the field decremented (subtract `1 << 31`) |
//! | normal, exponent field 1 | flushed to zero: `v = sign`, `az`, `an = sign`, `au` |
//! | zero or flush (field 0), infinity or invalid (field 255, no `av`) | `o` unchanged: no exponent to move |
//! | `av` (the sum overflowed, field 255 or the largest normal) | the halved sum is a normal number: `float::add(.., halve)` (cold, the sum of two numbers above 2^127) |
//!
//! The zero-operand pass-through of the add kernel (`(0, y)` returns `y`) needs no case of its
//! own: the reference halves `y` the same way (`round_pack(y.exp - 31 - 1, y.man)`).
//!
//! Provenance: VERIFIED bit-identical to `float::add(.., halve = true)` (value and every flag) by
//! `avg_equals_reference_on_the_class_grid_and_seeded_streams` and
//! `paired_avg_equals_scalar_calls_and_the_reference`.
use super::{finish, finish2, FlagsOut, Mode, F40};
use crate::core::aot_fast::fast_add;
use crate::core::aot_round::{pack, unpack, Packed};
use crate::core::aot_simd::fast_add2;
use crate::core::float::{self, Out};

/// Exponent field of a 40-bit float, bits 38..31.
/// VERIFIED: `float.rs` layout, checked by the grid tests below.
const FIELD_SHIFT: u32 = 31;
/// One unit of the exponent field (halving decrements it).
/// VERIFIED: halving is an exact scaling; checked by the grid tests below against `float::add`.
const ONE_EXP: u64 = 1 << FIELD_SHIFT;

/// The average from the sum `o` of `a + b` (see the table in the module doc).
#[inline(always)]
fn halved(o: Out, a: F40, b: F40, mode: Mode) -> Out {
    if o.av {
        return unpack(overflowed(a, b, mode));
    }
    let field = (o.v >> FIELD_SHIFT) & 0xFF;
    // 0 (zero, flush) and 255 (infinity, invalid): nothing to decrement
    if field.wrapping_sub(1) >= 254 {
        return o;
    }
    if field >= 2 {
        return Out {
            v: o.v - ONE_EXP,
            ..o
        };
    }
    // the sum is the smallest normal exponent: the halved value is below it and flushes
    Out {
        v: o.v & (1 << 39),
        az: true,
        an: o.an,
        au: true,
        ..Out::default()
    }
}

/// The reference average of a sum that overflowed (cold: both operands are above 2^126).
#[cold]
#[inline(never)]
fn overflowed(a: F40, b: F40, mode: Mode) -> Packed {
    pack(float::add(a, b, false, true, mode))
}

/// `(a + b) / 2`.
#[inline(always)]
pub fn favg(a: F40, b: F40, mode: Mode, st: &mut FlagsOut) -> F40 {
    finish(halved(fast_add(a, b, false, mode), a, b, mode), st)
}

/// `(a + b) / 2` on both elements: `[PEx, PEy]`.
#[inline(always)]
pub fn favg2(a: [F40; 2], b: [F40; 2], mode: Mode, st: &mut [FlagsOut; 2]) -> [F40; 2] {
    let o = fast_add2(a, b, false, mode);
    finish2(
        [
            halved(o[0], a[0], b[0], mode),
            halved(o[1], a[1], b[1], mode),
        ],
        st,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::kernels::f40;
    use crate::core::uop::kernels::grid::{self, Seen, AN, AU, AZ};
    use crate::core::uop::verify::gen::Rng;

    fn check(a: F40, b: F40, mode: Mode, seen: &mut Seen) {
        let mut st = grid::dirty();
        let v = favg(a, b, mode, &mut st);
        assert_eq!(
            st.out(v),
            float::add(a, b, false, true, mode),
            "a={a:#x} b={b:#x} {mode:?}"
        );
        seen.result(&st);
        seen.operand(a);
        seen.operand(b);
    }

    #[test]
    fn average_halves_the_sum_without_losing_range() {
        let (mut st, m) = (FlagsOut::default(), Mode::default());
        assert_eq!(favg(f40(3.0), f40(1.0), m, &mut st), f40(2.0));
        // the sum of the two largest singles overflows, the average does not
        let big = f40(f32::MAX);
        assert_eq!(favg(big, big, m, &mut st), big);
        assert!(!st.av);
    }

    #[test]
    fn the_edges_of_the_exponent_range_are_the_ones_the_table_names() {
        let (mut st, m) = (grid::dirty(), Mode::default());
        // smallest normal + smallest normal = 2^-125, halved 2^-126 (field 1): still normal
        let tiny = 1u64 << 31;
        assert_eq!(favg(tiny, tiny, m, &mut st), tiny);
        assert!(!st.au && !st.az);
        // the smallest normal averaged with zero halves to 2^-127: flushed, AU and AZ set
        assert_eq!(favg(tiny, 0, m, &mut st), 0);
        assert!(st.au && st.az && !st.an);
        assert_eq!(favg(1 << 39 | tiny, 0, m, &mut st), 1 << 39);
        assert!(st.au && st.az && st.an, "the flushed zero keeps the sign");
        // overflow of the sum: halved it is the largest finite value, TRUNC or not
        let max = 0xFE << 31 | 0x7FFF_FFFF;
        let t = Mode { trunc: true, ..m };
        assert_eq!(favg(max, max, t, &mut st), max);
        assert!(!st.av);
        // both zero, infinity with a normal, infinity against minus infinity
        assert_eq!(favg(0, 0, m, &mut st), 0);
        assert!(st.az);
        let inf = 0xFFu64 << 31;
        assert_eq!(favg(inf, f40(1.0), m, &mut st), inf);
        favg(inf, 1 << 39 | inf, m, &mut st);
        assert!(st.ai);
    }

    #[test]
    fn avg_equals_reference_on_the_class_grid_and_seeded_streams() {
        let mut seen = Seen::default();
        let g = grid::grid();
        for mode in grid::modes() {
            for &a in &g {
                for &b in &g {
                    check(a, b, mode, &mut seen);
                }
            }
        }
        let mut r = Rng::new(0xA760_0001);
        for mode in grid::modes() {
            for _ in 0..50_000 {
                let (a, b) = (grid::operand(&mut r), grid::operand(&mut r));
                check(a, b, mode, &mut seen);
                check(a, grid::near(&mut r, a), mode, &mut seen);
            }
            // exponents at both ends of the range, where the decrement changes the verdict
            for _ in 0..30_000 {
                let low = |r: &mut Rng| {
                    grid::operand(r) & !(0xFFu64 << 31) | (1 + r.next_u64() % 3) << 31
                };
                let high = |r: &mut Rng| {
                    grid::operand(r) & !(0xFFu64 << 31) | (252 + r.next_u64() % 3) << 31
                };
                let (a, b) = (low(&mut r), low(&mut r));
                check(a, b, mode, &mut seen);
                check(a, b ^ (1 << 39), mode, &mut seen);
                let (a, b) = (high(&mut r), high(&mut r));
                check(a, b, mode, &mut seen);
                check(a, b ^ (1 << 39), mode, &mut seen);
            }
        }
        // flush (AU) and zero (AZ) results must both have been compared, not just the main path
        seen.assert_reached(600_000, &[AZ, AN, AU]);
    }

    #[test]
    fn paired_avg_equals_scalar_calls_and_the_reference() {
        let mut seen = Seen::default();
        let g = grid::grid();
        let mut r = Rng::new(0xA760_0002);
        for mode in grid::modes() {
            for (i, &a0) in g.iter().enumerate() {
                for &b0 in &g {
                    let (a1, b1) = match i % 3 {
                        0 => (grid::operand(&mut r), grid::operand(&mut r)),
                        1 => (grid::near(&mut r, a0), b0),
                        _ => (b0, a0),
                    };
                    let (a, b) = ([a0, a1], [b0, b1]);
                    let mut st = [grid::dirty(); 2];
                    let v = favg2(a, b, mode, &mut st);
                    for lane in 0..2 {
                        let mut one = grid::dirty();
                        let s = favg(a[lane], b[lane], mode, &mut one);
                        assert_eq!((v[lane], st[lane]), (s, one), "lane {lane} {a:#x?} {b:#x?}");
                        seen.result(&st[lane]);
                    }
                    seen.operand(a0);
                    seen.operand(a1);
                }
            }
        }
        seen.assert_reached(300_000, &[AZ, AN, AU]);
    }
}
