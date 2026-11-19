//! The firmware reverb's number format: the DSP's 40-bit register ([`E40`]) or plain `f32`.
//!
//! [`Fp`] is the arithmetic the generated reverb steps (`fw_rvb_steps.rs`) and the half-rate
//! wrapper (`fw_rvb_wrap.rs`) are written against. Two implementations:
//!
//! * [`E40`]: every product and sum is rounded to 32 significant bits with ties to even, a store
//!   to a data word keeps the upper 32 bits (a truncation to 24 significant bits), and a result
//!   below the smallest normal single flushes to zero. VERIFIED word for word against the
//!   `sharc-fx` oracle (`tests_fw_rvb_golden.rs`, the 0x12417E wrapper end to end on the real
//!   reverb instance); the same rules as `sinc::ext40`, but exact: the integer mantissa product
//!   and sum avoid the double rounding of an `f64` detour.
//! * `f32`: the fast path, ordinary single precision (a register keeps 24 bits, stores do not
//!   truncate). Same data flow, error against the 40-bit path measured in `tests_fw_rvb_golden.rs`.
//!
//! Allocation free, no state.

/// A register-file number: operations, and the conversion to and from a 32-bit data word.
pub trait Fp: Copy + Default + std::fmt::Debug {
    /// A data word read into a register (a zero exponent field flushes to zero).
    fn from_word(w: u32) -> Self;
    /// The data word a store keeps (upper 32 bits of the register).
    fn to_word(self) -> u32;
    /// `a + b` rounded to the register format.
    fn add(self, o: Self) -> Self;
    /// `a - b`.
    fn sub(self, o: Self) -> Self;
    /// `a * b`.
    fn mul(self, o: Self) -> Self;
    /// `(a + b) / 2`, one rounding.
    fn avg(self, o: Self) -> Self;

    /// The register value as an `f64` (exact for both formats).
    fn to_f64(self) -> f64;
    /// A register value from an `f64` that is exact in the format (integers of at most 32 bits
    /// times a power of two, or a value taken from [`Fp::to_f64`]).
    fn from_f64(x: f64) -> Self;

    /// The value after a store to memory and a reload.
    fn store(self) -> Self {
        Self::from_word(self.to_word())
    }
    /// A float32 data word as a number.
    fn from_f32(x: f32) -> Self {
        Self::from_word(x.to_bits())
    }
    /// The float32 a store keeps.
    fn to_f32(self) -> f32 {
        f32::from_bits(self.to_word())
    }
}

/// Smallest normal single: smaller results flush to zero (VERIFIED: oracle, 60 calls); the
/// exponent test in `pack` is the same limit.
fn flush32(w: u32) -> u32 {
    if (w >> 23) & 0xFF == 0 {
        0
    } else {
        w
    }
}

impl Fp for f32 {
    fn from_word(w: u32) -> Self {
        f32::from_bits(flush32(w))
    }
    fn to_word(self) -> u32 {
        flush32(self.to_bits())
    }
    fn add(self, o: Self) -> Self {
        Self::from_word((self + o).to_bits())
    }
    fn sub(self, o: Self) -> Self {
        Self::from_word((self - o).to_bits())
    }
    fn mul(self, o: Self) -> Self {
        Self::from_word((self * o).to_bits())
    }
    fn avg(self, o: Self) -> Self {
        Self::from_word(((self + o) * 0.5).to_bits())
    }
    fn to_f64(self) -> f64 {
        f64::from(self)
    }
    fn from_f64(x: f64) -> Self {
        Self::from_word((x as f32).to_bits())
    }
}

/// A 40-bit register value: at most 32 significant bits, held exactly in an `f64`.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct E40(f64);

const MANT: u64 = (1 << 52) - 1;
/// The 21 low mantissa bits an `f64` holding a 32-bit value must show as zero, and the pattern
/// `1000...0` of a tie between two 32-bit neighbours.
const TIE_MASK: u64 = (1 << 21) - 1;
const TIE: u64 = 1 << 20;

/// `(negative, 32-bit mantissa with its top bit set, exponent of the mantissa's lsb)`.
fn unpack(x: f64) -> (bool, u64, i32) {
    let b = x.to_bits();
    let m = ((b & MANT) | (1 << 52)) >> 21;
    (b >> 63 != 0, m, ((b >> 52) & 0x7FF) as i32 - 1023 - 31)
}

/// `(-1)^neg * r * 2^lsb` rounded to 32 significant bits (ties to even); flush below [`TINY`].
fn pack(neg: bool, r: u128, lsb: i32) -> E40 {
    if r == 0 {
        return E40(0.0);
    }
    let n = 128 - r.leading_zeros() as i32;
    let (mut q, mut e) = if n > 32 {
        let sh = (n - 32) as u32;
        let (q, rest, half) = (r >> sh, r & ((1u128 << sh) - 1), 1u128 << (sh - 1));
        let up = rest > half || (rest == half && q & 1 == 1);
        (q + u128::from(up), lsb + sh as i32)
    } else {
        (r << (32 - n), lsb - (32 - n))
    };
    if q >> 32 != 0 {
        q >>= 1;
        e += 1;
    }
    let exp = e + 31;
    if exp < -126 {
        return E40(0.0);
    }
    let bits = (u64::from(neg) << 63) | (((exp + 1023) as u64) << 52) | (((q as u64) << 21) & MANT);
    E40(f64::from_bits(bits))
}

/// [`pack`] of a product of two 32-bit mantissas that both have their top bit set: `r` is in
/// `[2^62, 2^64)`, so the shift is 31..=32 and no 128-bit arithmetic is needed. Same rounding
/// (ties to even) and flush as [`pack`]; `tests::product_pack_equals_the_generic_pack`.
fn pack_product(neg: bool, r: u64, lsb: i32) -> E40 {
    let n = 64 - r.leading_zeros();
    let sh = n - 32;
    let (q, rest, half) = (r >> sh, r & ((1u64 << sh) - 1), 1u64 << (sh - 1));
    let mut q = q + u64::from(rest > half || (rest == half && q & 1 == 1));
    let mut e = lsb + sh as i32;
    if q >> 32 != 0 {
        q >>= 1;
        e += 1;
    }
    let exp = e + 31;
    if exp < -126 {
        return E40(0.0);
    }
    let bits = (u64::from(neg) << 63) | (((exp + 1023) as u64) << 52) | ((q << 21) & MANT);
    E40(f64::from_bits(bits))
}

/// Rounds an `f64` that holds an exact sum (at most 53 significant bits) to 32 significant bits,
/// ties to even, flushing a result below the smallest normal single; the same result as
/// [`pack`] on the sum's integer mantissa. A carry out of the mantissa increments the exponent
/// field, which is the renormalisation `pack` does by hand.
#[inline(always)]
fn round_exact(x: f64) -> E40 {
    const LOW: u64 = (1 << 21) - 1;
    let bits = x.to_bits();
    let (low, base) = (bits & LOW, bits & !LOW);
    let up = low > (1 << 20) || (low == (1 << 20) && base & (1 << 21) != 0);
    let r = base + if up { 1 << 21 } else { 0 };
    if ((r >> 52) & 0x7FF) < (1023 - 126) {
        return E40(0.0);
    }
    E40(f64::from_bits(r))
}

impl E40 {
    /// The value as an `f64` (exact).
    pub fn value(self) -> f64 {
        self.0
    }
}

impl Fp for E40 {
    fn from_word(w: u32) -> Self {
        E40(f64::from(f32::from_bits(flush32(w))))
    }
    fn to_word(self) -> u32 {
        if self.0 == 0.0 {
            return 0;
        }
        // keep 23 fraction bits of the 52: truncation toward zero, then exact to f32
        let t = f64::from_bits(self.0.to_bits() & !((1u64 << 29) - 1));
        flush32((t as f32).to_bits())
    }
    #[inline(always)]
    fn add(self, o: Self) -> Self {
        if self.0 == 0.0 {
            return o;
        }
        if o.0 == 0.0 {
            return self;
        }
        // Fast path 1: operands within 20 binades of each other: both have at most 32 significant
        // bits, so the sum spans at most 20 + 33 = 53 bits and the f64 sum is exact.
        let d = ((self.0.to_bits() >> 52) & 0x7FF).wrapping_sub((o.0.to_bits() >> 52) & 0x7FF);
        if d.wrapping_add(20) <= 40 {
            let s = self.0 + o.0;
            return if s == 0.0 { E40(0.0) } else { round_exact(s) };
        }
        self.add_far(o)
    }
    #[inline(always)]
    fn sub(self, o: Self) -> Self {
        self.add(E40(-o.0))
    }
    #[inline(always)]
    fn mul(self, o: Self) -> Self {
        if self.0 == 0.0 || o.0 == 0.0 {
            return E40(0.0);
        }
        // Fast path: the f64 product has one rounding (to 53 bits); rounding that to 32 bits is
        // the exact 32-bit result unless the f64 value sits exactly on a 32-bit tie, where the
        // lost low part decides. That case (about 2^-21 of the products) takes the integer path.
        let p = self.0 * o.0;
        if p.to_bits() & TIE_MASK != TIE {
            return round_exact(p);
        }
        self.mul_tie(o)
    }
    fn avg(self, o: Self) -> Self {
        let s = self.add(o);
        E40(s.0 * 0.5)
    }
    fn to_f64(self) -> f64 {
        self.0
    }
    fn from_f64(x: f64) -> Self {
        E40(x)
    }
}

impl E40 {
    /// Product on the integer mantissas (the f64 product sits on a 32-bit tie).
    #[cold]
    #[inline(never)]
    fn mul_tie(self, o: Self) -> Self {
        let (a, b) = (unpack(self.0), unpack(o.0));
        pack_product(a.0 != b.0, a.1 * b.1, a.2 + b.2)
    }

    /// Sum of operands more than 20 binades apart: exact in f64 only when the error term is zero.
    #[inline(never)]
    fn add_far(self, o: Self) -> Self {
        let s = self.0 + o.0;
        let bb = s - self.0;
        if (self.0 - (s - bb)) + (o.0 - bb) == 0.0 {
            return if s == 0.0 { E40(0.0) } else { round_exact(s) };
        }
        self.add_wide(o)
    }

    /// Both operands non-zero: the sum from the integer mantissas (any exponent distance).
    fn add_wide(self, o: Self) -> Self {
        let (a, b) = (unpack(self.0), unpack(o.0));
        // order by magnitude: x is the larger
        let a_big = (a.2, a.1) >= (b.2, b.1);
        let (x, y) = if a_big { (a, b) } else { (b, a) };
        let d = x.2 - y.2;
        if d > 70 {
            return if a_big { self } else { o };
        }
        let (mx, my) = ((u128::from(x.1)) << d, u128::from(y.1));
        let r = if x.0 == y.0 { mx + my } else { mx - my };
        pack(x.0, r, y.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(x: f64) -> E40 {
        E40(x)
    }

    #[test]
    fn exact_small_integers_and_signs() {
        assert_eq!(e(3.0).add(e(4.0)).value(), 7.0);
        assert_eq!(e(3.0).sub(e(5.0)).value(), -2.0);
        assert_eq!(e(-3.0).mul(e(4.0)).value(), -12.0);
        assert_eq!(e(1.0).avg(e(2.0)).value(), 1.5);
        assert_eq!(e(2.0).sub(e(2.0)).value(), 0.0);
    }

    #[test]
    fn products_round_to_32_bits_ties_to_even() {
        // (1 + 2^-31) * (1 + 2^-31) = 1 + 2^-30 + 2^-62: rounds to 1 + 2^-30
        let a = e(1.0 + 2f64.powi(-31));
        assert_eq!(a.mul(a).value(), 1.0 + 2f64.powi(-30));
        // 1 + 2^-32 is a tie between 1 and 1 + 2^-31: even is 1
        let t = e(1.0).add(e(2f64.powi(-32)));
        assert_eq!(t.value(), 1.0);
        // a tie that rounds up to the even neighbour: (1 + 2^-31) + 2^-32
        let u = e(1.0 + 2f64.powi(-31)).add(e(2f64.powi(-32)));
        assert_eq!(u.value(), 1.0 + 2f64.powi(-30));
    }

    #[test]
    fn a_much_smaller_operand_does_not_change_the_larger() {
        assert_eq!(e(1.0).add(e(1e-30)).value(), 1.0);
        assert_eq!(e(-1.0).add(e(1e-30)).value(), -1.0);
    }

    #[test]
    fn the_add_and_mul_match_exact_rational_rounding_on_random_values() {
        // reference: f64 with an exactly rounded 32-bit result via u128 big arithmetic
        let mut s = 0x1234_5678_9abc_def0_u64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..20_000 {
            let (a, b) = (next(), next());
            let fa = f32::from_bits(((a >> 32) as u32 & 0xBFFF_FFFF) | 0x2000_0000);
            let fb = f32::from_bits((b as u32 & 0xBFFF_FFFF) | 0x2000_0000);
            let (x, y) = (E40(f64::from(fa)), E40(f64::from(fb)));
            let p = x.mul(y).value();
            let exact = f64::from(fa) * f64::from(fb); // 48 bits: exact in f64
            let r = crate::sinc::ext40::round_sig(exact, 32, false);
            assert_eq!(p, r, "mul {fa} {fb}");
        }
    }

    /// Random operands with exponent spreads from 0 to 80 and cancellations.
    fn random_pairs(n: usize) -> Vec<(E40, E40)> {
        let mut s = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        (0..n)
            .map(|k| {
                let mant = |r: u64| 1.0 + f64::from((r >> 32) as u32) / 4_294_967_296.0;
                let (a, b) = (next(), next());
                let (ea, eb) = (
                    ((a & 0x3F) as i32) - 30,
                    ((a & 0x3F) as i32) - 30 - (next() % 81) as i32 * i32::from(k % 3 != 0),
                );
                let x = mant(a) * 2f64.powi(ea) * if a & 0x100 != 0 { -1.0 } else { 1.0 };
                let y = mant(b) * 2f64.powi(eb) * if b & 0x100 != 0 { -1.0 } else { 1.0 };
                // mantissas of 32 bits as the machine holds them
                (
                    pack(x < 0.0, (mant(a) * 2f64.powi(31)) as u128, ea - 31),
                    pack(y < 0.0, (mant(b) * 2f64.powi(31)) as u128, eb - 31),
                )
            })
            .collect()
    }

    #[test]
    fn the_exact_sum_fast_path_equals_the_integer_mantissa_sum() {
        for (a, b) in random_pairs(200_000) {
            assert_eq!(a.add(b), a.add_wide(b), "{a:?} + {b:?}");
            assert_eq!(a.sub(b), a.add_wide(E40(-b.0)), "{a:?} - {b:?}");
        }
        // a rounding carry into the exponent (1.111..1 + a tie bit), a cancellation to exactly
        // +0, and a result below the smallest normal single flushing to zero
        let near_two = E40(2.0 - 2f64.powi(-31));
        let bit = E40(2f64.powi(-32));
        assert_eq!(near_two.add(bit), near_two.add_wide(bit));
        assert_eq!(E40(1.5).sub(E40(1.5)).value().to_bits(), 0);
        let (a, b) = (E40(1.5 * 2f64.powi(-126)), E40(2f64.powi(-126)));
        assert_eq!(a.sub(b).value(), 0.0, "0.5 * 2^-126 is below the limit");
        assert_eq!(a.sub(b), a.add_wide(E40(-b.0)));
    }

    #[test]
    fn product_pack_equals_the_generic_pack() {
        for (a, b) in random_pairs(200_000) {
            let (x, y) = (unpack(a.0), unpack(b.0));
            let want = pack(x.0 != y.0, u128::from(x.1 * y.1), x.2 + y.2);
            assert_eq!(a.mul(b), want, "{a:?} * {b:?}");
        }
    }

    #[test]
    fn store_truncates_to_24_bits_and_flushes_the_tiny() {
        let x = e(1.0 + 2f64.powi(-23) + 2f64.powi(-30));
        assert_eq!(x.to_word(), (1.0f32 + 2f32.powi(-23)).to_bits());
        assert_eq!(e(-0.0).to_word(), 0);
        assert_eq!(
            E40::from_word(0x0000_0001).value(),
            0.0,
            "denormal word flushes"
        );
        let small = e(2f64.powi(-100)).mul(e(2f64.powi(-100)));
        assert_eq!(small.value(), 0.0, "below the smallest normal");
    }

    #[test]
    fn f32_implementation_flushes_and_averages() {
        assert_eq!(<f32 as Fp>::add(1.0, 2.0), 3.0);
        assert_eq!(<f32 as Fp>::avg(1.0, 2.0), 1.5);
        assert_eq!(<f32 as Fp>::mul(1e-30, 1e-30), 0.0);
        assert_eq!(<f32 as Fp>::from_word(0x0000_0010), 0.0);
        assert_eq!(1.5f32.store(), 1.5);
        assert_eq!(E40::from_f32(0.25).to_f32(), 0.25);
    }
}

#[cfg(test)]
mod fast_tests;
