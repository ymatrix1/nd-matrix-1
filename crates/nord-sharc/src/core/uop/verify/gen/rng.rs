//! The seeded generator and the register values that sit on a boundary. OWNER: W1-VERIFY.

/// xorshift64* (Vigna 2016), seeded. ASSUMED: any full-period generator does; it only has to be
/// reproducible.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator for `seed` (zero is replaced by a fixed non-zero value).
    pub fn new(seed: u64) -> Self {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// The next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A value in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }

    /// One element of `xs` (`xs` must not be empty).
    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u32) as usize]
    }
}

/// Register values that sit on a boundary of the float and fixed-point rules: zero, negative
/// zero, denormals, one, the largest and smallest normals, infinities, NaN, the sign bit, ones.
/// ASSUMED: the IEEE 754 binary32 boundaries (the SHARC 32-bit float format is binary32 with its
/// own denormal and NaN handling); a boundary the model treats specially that this list misses
/// would show as a family fuzz gap, and `every_class_is_detected_from_any_start_state...` does not
/// depend on it. Every entry is drawn by `below_and_pick_stay_in_range_and_edge_words_cover_the_boundaries`.
pub const EDGE_WORDS: [u32; 14] = [
    0x0000_0000,
    0x8000_0000,
    0x0000_0001,
    0x807F_FFFF,
    0x0080_0000,
    0x3F80_0000,
    0xBF80_0000,
    0x7F7F_FFFF,
    0x7F80_0000,
    0xFF80_0000,
    0x7FC0_0000,
    0x7FFF_FFFF,
    0xFFFF_FFFF,
    0x0000_FFFF,
];

/// A register value: an edge word half of the time, otherwise random bits with a normal exponent.
/// ASSUMED: biased exponents 110..146 (values near 2^-17..2^18) keep sums and products of two
/// draws inside the normal range, so the float paths see ordinary arithmetic and not only
/// overflow; checked by `random_edge_words_have_a_normal_exponent_when_they_are_not_an_edge`.
pub fn edge_word(rng: &mut Rng) -> u32 {
    if rng.below(2) == 0 {
        EDGE_WORDS[rng.below(EDGE_WORDS.len() as u32) as usize]
    } else {
        let v = rng.next_u32();
        (v & 0x807F_FFFF) | ((110 + rng.below(36)) << 23)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_reproduces_the_same_stream_and_zero_is_a_valid_seed() {
        let (mut a, mut b) = (Rng::new(42), Rng::new(42));
        let xs: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        assert_eq!(xs, (0..8).map(|_| b.next_u64()).collect::<Vec<_>>());
        assert_ne!(Rng::new(0).next_u64(), 0);
        assert!(xs.windows(2).all(|w| w[0] != w[1]));
    }

    #[test]
    fn the_stream_is_pinned_so_a_seed_means_the_same_failure_on_every_platform() {
        // VERIFIED: the first outputs of xorshift64* (shifts 12, 25, 27, multiplier
        // 0x2545F4914F6CDD1D) for seed 1 and for seed 0 (replaced by 0x9E3779B97F4A7C15), computed
        // independently in Python. A change here invalidates every recorded seed.
        let mut r = Rng::new(1);
        let first: Vec<u64> = (0..3).map(|_| r.next_u64()).collect();
        assert_eq!(
            first,
            [
                0x47E4_CE4B_896C_DD1D,
                0xABCF_A6A8_E079_651D,
                0xB9D1_0D8F_EB73_1F57
            ]
        );
        assert_eq!(Rng::new(0).next_u64(), 0x0D83_B3E2_9A21_487A);
        assert_eq!(
            Rng::new(7).next_u32(),
            (Rng::new(7).next_u64() >> 32) as u32
        );
    }

    #[test]
    fn below_and_pick_stay_in_range_and_edge_words_cover_the_boundaries() {
        let mut r = Rng::new(7);
        assert!((0..1000).all(|_| r.below(14) < 14));
        let seen: std::collections::BTreeSet<u32> = (0..2000).map(|_| edge_word(&mut r)).collect();
        assert!(EDGE_WORDS.iter().all(|e| seen.contains(e)));
        let xs = [3u8, 5, 9];
        let picked: std::collections::BTreeSet<u8> = (0..200).map(|_| r.pick(&xs)).collect();
        assert_eq!(picked.into_iter().collect::<Vec<_>>(), xs);
    }

    #[test]
    fn random_edge_words_have_a_normal_exponent_when_they_are_not_an_edge() {
        let mut r = Rng::new(9);
        for _ in 0..500 {
            let w = edge_word(&mut r);
            let exp = w >> 23 & 0xFF;
            assert!(
                EDGE_WORDS.contains(&w) || (110..146).contains(&exp),
                "{w:#x}"
            );
        }
    }
}
