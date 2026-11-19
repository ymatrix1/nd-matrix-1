//! Tests of the `E40` fast paths (`add` within 20 binades, `mul` through the f64 product) against
//! the integer mantissa arithmetic they replace.

use super::*;

fn e(x: f64) -> E40 {
    E40(x)
}

/// Both operands at their widest, a carry out of the top, the small one's lowest bit set: the
/// sum needs `d + 33` bits, so `d = 20` is the last exponent distance the f64 sum holds exactly
/// and `d = 21` is the first that does not.
#[test]
fn the_exponent_distance_fast_path_stops_where_the_f64_sum_stops_being_exact() {
    let widest = |exp: i32| e((2f64.powi(32) - 1.0) * 2f64.powi(exp - 31));
    for d in 15..=26 {
        for sign in [1.0, -1.0] {
            let (x, y) = (
                widest(0),
                e(sign * (2f64.powi(32) - 1.0) * 2f64.powi(-d - 31)),
            );
            assert_eq!(x.add(y), x.add_wide(y), "d = {d}, sign {sign}");
            assert_eq!(y.add(x), x.add_wide(y), "d = {d}, sign {sign}, swapped");
            assert_eq!(x.sub(y), x.add_wide(E40(-y.0)), "d = {d} sub");
        }
    }
}

/// A 17-bit odd factor times a 16-bit odd one has 32 or 33 significant bits: 33 bits is exactly
/// a tie between two 32-bit neighbours (the f64 product is exact, so its low 21 bits read
/// `1000...0`), which must round to even; every other product must still equal the integer path.
#[test]
fn the_product_fast_path_resolves_ties_like_the_integer_path() {
    let mut s = 0xdead_beef_cafe_f00d_u64;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut ties = 0;
    for _ in 0..100_000_u32 {
        let (a, b) = (next(), next());
        let m17 = (a as u32 & 0x1_FFFF) | 0x1_0001; // top bit and low bit set: exactly 17 bits
        let m16 = (b as u32 & 0xFFFF) | 0x8001;
        let sh = (a >> 40) as u32 % 14; // moves the factors apart, still at most 32 bits each
        let x = E40(f64::from(m17 << sh) * 2f64.powi(-20 - (a >> 50) as i32 % 9));
        let y = E40(f64::from(m16) * 2f64.powi(-15 - (b >> 50) as i32 % 9));
        let (p, q) = (unpack(x.0), unpack(y.0));
        let want = pack(p.0 != q.0, u128::from(p.1 * q.1), p.2 + q.2);
        ties += usize::from((x.0 * y.0).to_bits() & TIE_MASK == TIE);
        assert_eq!(x.mul(y), want, "{x:?} * {y:?}");
    }
    assert!(ties > 10_000, "the stimulus reaches the tie path: {ties}");
    let a = e(1.0 + 2f64.powi(-16));
    assert_eq!(a.mul(a).value(), 1.0 + 2f64.powi(-15), "tie, even is down");
    let b = e(1.0 + 3.0 * 2f64.powi(-16));
    // 9 * 2^-32 = 2^-29 + 2^-32: a tie between 4 and 5 ulps (2^-31), even is 4
    assert_eq!(
        b.mul(b).value(),
        1.0 + 3.0 * 2f64.powi(-15) + 2f64.powi(-29),
        "tie, even"
    );
}
