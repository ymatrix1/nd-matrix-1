//! Equivalence sweeps of the paired kernels (`aot_simd.rs`) against the reference interpreter
//! kernels (`float.rs`): random operand classes with independent lanes, directed grids (every
//! exponent gap, rounding ties, overflow and flush edges, cancellation) with the cases of one lane
//! paired against a different case in the other lane, every lane combination of normal and
//! special operands, and all four TRUNC x RND32 modes. Every `Out` field is compared.
//!
//! The same checker runs deliberately broken kernels (swapped lanes, a wrong or missing
//! fallback) and must reject them: that is the mutation check of the sweep itself.
//!
//! Provenance: VERIFIED bit-identical to `float::add` / `float::mul` on every case below (the
//! tests are the check); the reference kernels are VERIFIED against the CCES simulator, see
//! `float.rs`.
use super::aot_fast::{fast_add, fast_mul};
use super::aot_simd::{add2_with, fast_add2, fast_mul2, fast_sub2, mul2_with, scalar};
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

type Add2 = dyn Fn([u64; 2], [u64; 2], bool, Mode) -> [Out; 2];
type Mul2 = dyn Fn([u64; 2], [u64; 2], Mode) -> [Out; 2];

/// One lane pair of operands against the reference, every mode; the first mismatch as text.
fn verify(a: [u64; 2], b: [u64; 2], add2: &Add2, mul2: &Mul2) -> Result<(), String> {
    for mode in modes() {
        for sub in [false, true] {
            let want = [0, 1].map(|i| float::add(a[i], b[i], sub, false, mode));
            let got = add2(a, b, sub, mode);
            if got != want {
                return Err(format!("add a={a:#x?} b={b:#x?} sub={sub} {mode:?}"));
            }
        }
        let want = [0, 1].map(|i| float::mul(a[i], b[i], mode));
        if mul2(a, b, mode) != want {
            return Err(format!("mul a={a:#x?} b={b:#x?} {mode:?}"));
        }
    }
    Ok(())
}

fn real_add2(a: [u64; 2], b: [u64; 2], sub: bool, m: Mode) -> [Out; 2] {
    if sub {
        fast_sub2(a, b, m)
    } else {
        fast_add2(a, b, false, m)
    }
}

/// The checker with the production kernels, panicking on the first mismatch.
fn check(a: [u64; 2], b: [u64; 2]) {
    if let Err(e) = verify(a, b, &real_add2, &fast_mul2) {
        panic!("{e}");
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

/// Runs every directed scalar pair against its neighbour and its mirror in the other lane, so a
/// case is checked next to a different class in lane 0 and in lane 1.
fn check_cases(cases: &[(u64, u64)]) {
    for (i, &(a0, b0)) in cases.iter().enumerate() {
        let (a1, b1) = cases[(i * 7 + 3) % cases.len()];
        check([a0, a1], [b0, b1]);
        check([a1, a0], [b1, b0]);
    }
}

#[test]
fn random_sweep_with_independent_lanes_matches_the_reference() {
    let mut r = Rng(0xA11C_E000_1234_5678);
    let exp_of = |r: &mut Rng, w: u64| match w % 8 {
        0 => 1 + r.next() % 3,
        1 => 252 + r.next() % 3,
        2 | 3 => 120 + r.next() % 16,
        4 => r.next() % 256,
        _ => 1 + r.next() % 254,
    };
    for _ in 0..300_000 {
        let v = r.next();
        let mut op = |w: u64| {
            let (e, f) = (exp_of(&mut r, w), r.next());
            pack(f >> 63 == 1, e, f >> 8, f)
        };
        let a = [op(v), op(v >> 8)];
        let b = [op(v >> 16), op(v >> 24)];
        check(a, b);
    }
}

#[test]
fn every_exponent_gap_and_fraction_pair_matches_in_both_lanes() {
    let mut cases = Vec::new();
    for gap in 0..=70u64 {
        for fa in FRACS {
            for fb in FRACS {
                for (sa, sb) in [(false, false), (false, true), (true, false), (true, true)] {
                    let big = pack(sa, 140 + gap.min(100), fa, 0x80);
                    let small = pack(sb, 140, fb, 0xFF);
                    cases.push((big, small));
                    cases.push((small, big));
                }
            }
        }
    }
    check_cases(&cases);
}

#[test]
fn gaps_around_the_exact_double_limit_match() {
    let smalls: Vec<u64> = (0..31)
        .flat_map(|k| [1u64 << k, (1 << k) | 1, (1u64 << k) - 1])
        .chain(FRACS)
        .collect();
    let mut cases = Vec::new();
    for gap in 19..=23u64 {
        for fa in FRACS {
            for &fb in &smalls {
                for (sa, sb) in [(false, false), (false, true), (true, false), (true, true)] {
                    let (big, small) = (pack(sa, 140 + gap, fa, 0), pack(sb, 140, fb, 0));
                    cases.push((big, small));
                    cases.push((small, big));
                }
            }
        }
    }
    check_cases(&cases);
}

#[test]
fn single_bit_significands_hit_every_tie_position() {
    let mut cases = Vec::new();
    for gap in 0..=40u64 {
        for p in 0..31 {
            for fa in FRACS {
                let a = pack(false, 150, fa, 0);
                let b = pack(false, 150 - gap.min(100), 1 << p, 0);
                cases.extend([(a, b), (b, a), (a | 1 << 39, b)]);
            }
        }
    }
    for i in 0..31 {
        for j in 0..31 {
            let (a, b) = (pack(false, 127, 1 << i, 0), pack(false, 127, 1 << j, 0));
            cases.extend([(a, b), (a | 1 << 39, b)]);
        }
    }
    check_cases(&cases);
}

#[test]
fn exponent_edges_give_the_same_overflow_and_flush_results() {
    let exps = [0, 1, 2, 3, 63, 64, 126, 127, 128, 190, 252, 253, 254, 255];
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
    let mut cases = Vec::new();
    for &a in &ops {
        for &b in ops.iter().step_by(5) {
            cases.push((a, b));
        }
    }
    check_cases(&cases);
}

#[test]
fn cancellation_to_every_leading_bit_position_matches() {
    let mut r = Rng(0xFACE_B00C_0000_0002);
    let mut cases = Vec::new();
    for _ in 0..100_000 {
        let v = r.next();
        let e = 1 + r.next() % 254;
        let base = pack(false, e, v >> 8, v);
        let k = r.next() % 40;
        let other = base ^ (r.next() & ((1u64 << k) - 1)) & 0xFF_FFFF_FFFF;
        cases.push((base, other | 1 << 39));
        cases.push((base, other));
    }
    check_cases(&cases);
}

#[test]
fn every_lane_combination_of_normal_and_special_operands_matches() {
    let one = pack(false, 127, 0x1234_5678, 0x5A);
    let kinds = [
        one,
        pack(true, 1, 0, 0),
        pack(false, 254, 0x7FFF_FFFF, 0xFF),
        0,
        pack(true, 0, 0x7F, 0),
        pack(false, 255, 0, 0),
        pack(false, 255, 5, 0x11),
        pack(true, 127, 0x7FFF_FFFF, 0),
    ];
    for &a0 in &kinds {
        for &a1 in &kinds {
            for &b0 in &kinds {
                for &b1 in &kinds {
                    check([a0, a1], [b0, b1]);
                }
            }
        }
    }
}

#[test]
fn scalar_pair_kernels_equal_the_reference() {
    let mut r = Rng(0x5CA1_AB1E_0000_0003);
    for _ in 0..50_000 {
        let v: [u64; 4] = std::array::from_fn(|_| pack(false, r.next() % 256, r.next(), r.next()));
        let add = |a, b, s, m| scalar::add2(a, b, s, m);
        let mul = |a, b, m| scalar::mul2(a, b, m);
        let (a, b) = ([v[0], v[1]], [v[2], v[3]]);
        assert_eq!(verify(a, b, &add, &mul), Ok(()));
    }
}

/// Mutation check: the sweep must reject a pair kernel that swaps its lanes, one whose fallback
/// is missing, one whose fallback ignores the rounding mode, and one that applies the wrong
/// subtraction in the fallback. A checker that passed these would not prove the real kernels.
#[test]
fn the_sweep_rejects_wrong_lanes_and_wrong_fallbacks() {
    let swapped_add = |a, b, s, m| {
        let [x, y] = fast_add2(a, b, s, m);
        [y, x]
    };
    let swapped_mul = |a, b, m| {
        let [x, y] = fast_mul2(a, b, m);
        [y, x]
    };
    let no_fallback_add = |a, b, s, m| add2_with(a, b, s, m, |_, _, _, _| Out::default());
    let no_fallback_mul = |a, b, m| mul2_with(a, b, m, |_, _, _, _| Out::default());
    let wrong_mode_add =
        |a, b, s, m| add2_with(a, b, s, m, |x, y, s, _| fast_add(x, y, s, Mode::default()));
    let wrong_mode_mul = |a, b, m| mul2_with(a, b, m, |x, y, _, _| fast_mul(x, y, Mode::default()));
    let wrong_sub_add = |a, b, s, m| add2_with(a, b, s, m, |x, y, s, m| fast_add(x, y, !s, m));
    let good_mul = |a, b, m| fast_mul2(a, b, m);
    let good_add = |a: [u64; 2], b: [u64; 2], s, m| fast_add2(a, b, s, m);

    let mut probes = Vec::new();
    let mut r = Rng(0x0BAD_CAFE_0000_0004);
    for _ in 0..4000 {
        let v: [u64; 4] =
            std::array::from_fn(|_| pack(r.next() >> 63 == 1, r.next() % 256, r.next(), r.next()));
        probes.push(([v[0], v[1]], [v[2], v[3]]));
    }
    let rejects = |add2: &Add2, mul2: &Mul2| {
        probes
            .iter()
            .any(|&(a, b)| verify(a, b, add2, mul2).is_err())
    };
    assert!(
        rejects(&swapped_add, &good_mul),
        "swapped add lanes accepted"
    );
    assert!(
        rejects(&good_add, &swapped_mul),
        "swapped mul lanes accepted"
    );
    assert!(
        rejects(&no_fallback_add, &good_mul),
        "missing add fallback accepted"
    );
    assert!(
        rejects(&good_add, &no_fallback_mul),
        "missing mul fallback accepted"
    );
    assert!(
        rejects(&wrong_mode_add, &good_mul),
        "add fallback ignoring mode accepted"
    );
    assert!(
        rejects(&good_add, &wrong_mode_mul),
        "mul fallback ignoring mode accepted"
    );
    assert!(
        rejects(&wrong_sub_add, &good_mul),
        "add fallback with wrong sub accepted"
    );
    // and the unmutated kernels pass the same probes
    assert!(!rejects(&good_add, &good_mul));
}
