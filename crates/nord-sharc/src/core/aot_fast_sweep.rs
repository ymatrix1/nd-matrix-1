//! Equivalence sweeps of the native float kernels (`aot_fast.rs`) against the reference
//! interpreter kernels (`float.rs`): a large deterministic pseudo-random sweep over operand
//! classes, and directed grids for rounding ties, carries out of the significand, cancellation,
//! every exponent gap, and the overflow and flush edges. All four TRUNC x RND32 modes, every
//! `Out` field compared (flags, sticky-exception inputs and value).
//!
//! Provenance: VERIFIED bit-identical to `float::add` / `float::mul` on every case below (the
//! tests are the check); the reference kernels are VERIFIED against the CCES simulator, see
//! `float.rs`.
use super::aot_fast::{fast_add, fast_mul};
use super::float::{self, Mode, Out};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn modes() -> [Mode; 4] {
    [(false, false), (true, false), (false, true), (true, true)].map(|(trunc, r32)| Mode {
        trunc,
        r32,
        sat: false,
    })
}

fn pack(neg: bool, exp: u64, frac: u64, ext: u64) -> u64 {
    u64::from(neg) << 39 | exp << 31 | (frac & 0x7FFF_FFFF) | ext & 0xFF
}

/// `fast_add` and `fast_mul` against the reference on one pair, every mode.
fn check_pair(a: u64, b: u64) {
    for mode in modes() {
        let want_mul: Out = float::mul(a, b, mode);
        assert_eq!(
            fast_mul(a, b, mode),
            want_mul,
            "mul a={a:#x} b={b:#x} {mode:?}"
        );
        for sub in [false, true] {
            assert_eq!(
                fast_add(a, b, sub, mode),
                float::add(a, b, sub, false, mode),
                "add a={a:#x} b={b:#x} sub={sub} {mode:?}"
            );
        }
    }
}

/// Fractions that sit on rounding boundaries, carries and single-bit patterns.
const FRACS: [u64; 10] = [
    0,
    1,
    2,
    0x4000_0000,
    0x3FFF_FFFF,
    0x7FFF_FFFF,
    0x7FFF_FFFE,
    0x5555_5555,
    0x2AAA_AAAA,
    0x0000_0100,
];

#[test]
fn random_sweep_over_operand_classes_matches_the_reference() {
    let mut r = Rng(0xC0FF_EE00_1234_5678);
    for _ in 0..600_000 {
        let v = r.next();
        let exp_of = |r: &mut Rng, w: u64| match w % 8 {
            0 => 1 + r.next() % 3,
            1 => 252 + r.next() % 3,
            2 | 3 => 120 + r.next() % 16,
            _ => 1 + r.next() % 254,
        };
        let (ea, eb) = (exp_of(&mut r, v), exp_of(&mut r, v >> 8));
        let (fa, fb) = (r.next(), r.next());
        let (a, b) = (
            pack(v >> 63 == 1, ea, fa >> 8, fa),
            pack(v >> 62 & 1 == 1, eb, fb >> 8, fb),
        );
        check_pair(a, b);
    }
}

#[test]
fn every_exponent_gap_and_fraction_pair_matches_for_add_and_sub() {
    for gap in 0..=70u64 {
        for fa in FRACS {
            for fb in FRACS {
                for (sa, sb) in [(false, false), (false, true), (true, false), (true, true)] {
                    // the larger operand is first, then second: both orders
                    let (big, small) = (
                        pack(sa, 140 + gap.min(100), fa, 0x80),
                        pack(sb, 140, fb, 0xFF),
                    );
                    check_pair(big, small);
                    check_pair(small, big);
                }
            }
        }
    }
}

/// Gaps 19..=23 straddle the limit (20) up to which the double sum is exact: sparse fractions
/// whose low bits are set make the 54-bit sum of gap 21 and above tie in the double, so a wrong
/// limit (21 or more) is caught by a mismatch here, for add and sub in every mode.
#[test]
fn gaps_around_the_exact_double_limit_match_in_both_orders_and_all_modes() {
    let smalls: Vec<u64> = (0..31)
        .flat_map(|k| [1u64 << k, (1 << k) | 1, (1u64 << k) - 1])
        .chain(FRACS)
        .collect();
    for gap in 19..=23u64 {
        for fa in FRACS {
            for &fb in &smalls {
                for (sa, sb) in [(false, false), (false, true), (true, false), (true, true)] {
                    let (big, small) = (pack(sa, 140 + gap, fa, 0), pack(sb, 140, fb, 0));
                    check_pair(big, small);
                    check_pair(small, big);
                }
            }
        }
    }
}

#[test]
fn single_bit_significands_hit_every_tie_position_in_add_and_mul() {
    for gap in 0..=40u64 {
        for p in 0..31 {
            for fa in FRACS {
                let (a, b) = (
                    pack(false, 150, fa, 0),
                    pack(false, 150 - gap.min(100), 1 << p, 0),
                );
                check_pair(a, b);
                check_pair(b, a);
                check_pair(a | 1 << 39, b);
            }
        }
    }
    for i in 0..31 {
        for j in 0..31 {
            let (a, b) = (pack(false, 127, 1 << i, 0), pack(false, 127, 1 << j, 0));
            check_pair(a, b);
            check_pair(a | 1 << 39, b);
        }
    }
}

#[test]
fn exponent_edges_give_the_same_overflow_and_flush_results() {
    let exps = [1, 2, 3, 63, 64, 126, 127, 128, 190, 252, 253, 254];
    let ops: Vec<u64> = exps
        .iter()
        .flat_map(|&e| {
            FRACS.iter().flat_map(move |&f| {
                [false, true]
                    .into_iter()
                    .flat_map(move |neg| [0u64, 0x80, 0xFF].map(move |x| pack(neg, e, f, x)))
            })
        })
        .collect();
    for &a in &ops {
        for &b in &ops {
            check_pair(a, b);
        }
    }
}

#[test]
fn cancellation_to_every_leading_bit_position_matches() {
    let mut r = Rng(0xFACE_B00C_0000_0001);
    for _ in 0..200_000 {
        let v = r.next();
        let e = 1 + r.next() % 254;
        let base = pack(false, e, v >> 8, v);
        // differ only in the low `k` bits, so the difference has a leading bit at any position
        let k = r.next() % 40;
        let other = base ^ (r.next() & ((1u64 << k) - 1)) & 0xFF_FFFF_FFFF;
        check_pair(base, other | 1 << 39);
        check_pair(base, other);
    }
}
