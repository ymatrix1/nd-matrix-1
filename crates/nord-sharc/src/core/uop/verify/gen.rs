//! Deterministic generators for the differential fuzz: a seeded PRNG, edge-biased register
//! values and random programs drawn from a pool of valid encodings.
//!
//! OWNER: W1-VERIFY (state and operand generators), W3-FUZZ (per-family program shapes).
//! No `rand` dependency: the xorshift generator below makes a seed reproduce a failure on every
//! platform. Skeleton: complete for the pool of encodings and the state filler; a family draws
//! its programs from its own table of encodings through [`random_program_from`] (it never edits
//! [`PROGRAM_POOL`]; W1-VERIFY adds a corpus of encodings per instruction kind taken from the
//! measured census, `docs/research/sharc-seq/`, for the families to filter).
use crate::core::Core;

/// Short-word address where [`core_with_code`] places a program (inside the VISA code window,
/// the same place the unit-test helper `support::core_with` uses).
pub const CODE_PC: u32 = 0x12_0000;

/// A fresh core with `program` (parcels, most significant first) at [`CODE_PC`] and the pc there.
/// Available outside tests, for the fuzz, sweep and report tools.
pub fn core_with_code(program: &[u16]) -> Core {
    let mut c = Core::new(&[]);
    for (k, p) in program.iter().enumerate() {
        let o = (CODE_PC as usize + k) * 2;
        c.sram[o..o + 2].copy_from_slice(&p.to_le_bytes());
    }
    c.pc = CODE_PC;
    c
}

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
}

/// Register values that sit on a boundary of the float and fixed-point rules: zero, negative
/// zero, denormals, one, the largest and smallest normals, infinities, NaN, the sign bit, ones.
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
pub fn edge_word(rng: &mut Rng) -> u32 {
    if rng.below(2) == 0 {
        EDGE_WORDS[rng.below(EDGE_WORDS.len() as u32) as usize]
    } else {
        let v = rng.next_u32();
        (v & 0x807F_FFFF) | ((110 + rng.below(36)) << 23)
    }
}

/// Fills the data registers, the multiplier registers, the float extensions, the flags and the
/// float MODE1 bits of `c` from `rng`. `simd` sets PEy enable. Address registers are left alone:
/// the pool programs below do no addressing.
pub fn fill_state(c: &mut Core, rng: &mut Rng, simd: bool) {
    for k in 0..16 {
        c.r[k] = edge_word(rng);
        c.s[k] = edge_word(rng);
        if rng.below(4) == 0 {
            c.fx.tag[k] = c.r[k];
            c.fx.lo[k] = rng.next_u32() as u8;
        }
        if rng.below(4) == 0 {
            c.fx_y.tag[k] = c.s[k];
            c.fx_y.lo[k] = rng.next_u32() as u8;
        }
    }
    c.mr = [
        i128::from(rng.next_u64() as i64),
        i128::from(rng.next_u64() as i64),
    ];
    c.mr_y = [
        i128::from(rng.next_u64() as i64),
        i128::from(rng.next_u64() as i64),
    ];
    let bits = rng.next_u32();
    c.flags.az = bits & 1 != 0;
    c.flags.an = bits & 2 != 0;
    c.flags.av = bits & 4 != 0;
    c.flags.ac = bits & 8 != 0;
    c.flags_y.az = bits & 16 != 0;
    c.flags_y.an = bits & 32 != 0;
    // MODE1: TRUNC 15, RND32 16, ALUSAT 13 (VERIFIED in float.rs), PEYEN 21
    let float_bits = (1 << 13) | (1 << 15) | (1 << 16);
    c.mode1 = (c.mode1 & !(float_bits | (1 << 21))) | (rng.next_u32() & float_bits);
    if simd {
        c.mode1 |= 1 << 21;
    }
}

/// Valid encodings (parcels of one instruction each), taken from the AOT lowering test: an
/// immediate load, fixed and float ALU and multiplier forms in 16, 32 and 48 bits, a
/// multifunction, register moves, a bit operation and a NOP.
pub const PROGRAM_POOL: &[&[u16]] = &[
    &[0x0f0e, 0x3eb0, 0xf34d],
    &[0x0180, 0x1ecc],
    &[0x018d, 0xa4ce],
    &[0x0188, 0x2be4],
    &[0x018e, 0x3bbe],
    &[0xc8be],
    &[0x0193, 0x0ebe],
    &[0x703e, 0x030c, 0x4040],
    &[0x01d9, 0x602e],
    &[0x703f, 0x863f],
    &[0x023e, 0x0032, 0x1fa0],
    &[0x018e, 0x000a],
    &[0x013e, 0x0000, 0x1312],
    &[0x0001],
];

/// The parcels of `n` instructions drawn from [`PROGRAM_POOL`].
pub fn random_program(rng: &mut Rng, n: usize) -> Vec<u16> {
    random_program_from(rng, PROGRAM_POOL, n)
}

/// The parcels of `n` instructions drawn from `pool` (a table of whole instructions, each one a
/// slice of parcels). Every family keeps its own table in its own test module and calls this, so
/// no package edits [`PROGRAM_POOL`] (a shared table every package would append to conflicts at
/// merge). `pool` must not be empty.
pub fn random_program_from(rng: &mut Rng, pool: &[&[u16]], n: usize) -> Vec<u16> {
    (0..n)
        .flat_map(|_| pool[rng.below(pool.len() as u32) as usize].iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::compare::exact_diff;

    #[test]
    fn core_with_code_places_the_program_where_the_test_helper_does() {
        let (a, b) = (
            core_with_code(&[0x1234, 0x5678]),
            crate::core::support::core_with(&[0x1234, 0x5678]),
        );
        assert_eq!(CODE_PC, crate::core::support::CODE);
        assert_eq!(exact_diff(&a, &b), None);
        assert_eq!(
            (a.parcel_at(CODE_PC), a.parcel_at(CODE_PC + 1)),
            (0x1234, 0x5678)
        );
    }

    #[test]
    fn a_seed_reproduces_the_same_stream_and_zero_is_a_valid_seed() {
        let (mut a, mut b) = (Rng::new(42), Rng::new(42));
        let xs: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        assert_eq!(xs, (0..8).map(|_| b.next_u64()).collect::<Vec<_>>());
        assert_ne!(Rng::new(0).next_u64(), 0);
        assert!(xs.windows(2).all(|w| w[0] != w[1]));
    }

    #[test]
    fn below_stays_in_range_and_edge_words_cover_the_boundaries() {
        let mut r = Rng::new(7);
        assert!((0..1000).all(|_| r.below(14) < 14));
        let seen: std::collections::BTreeSet<u32> = (0..2000).map(|_| edge_word(&mut r)).collect();
        assert!(EDGE_WORDS.iter().all(|e| seen.contains(e)));
    }

    #[test]
    fn fill_state_is_deterministic_and_honours_simd() {
        let (mut a, mut b) = (Core::new(&[]), Core::new(&[]));
        fill_state(&mut a, &mut Rng::new(3), true);
        fill_state(&mut b, &mut Rng::new(3), true);
        assert_eq!(exact_diff(&a, &b), None);
        assert!(a.simd());
        fill_state(&mut b, &mut Rng::new(3), false);
        assert!(!b.simd());
        assert_ne!(a.r, [0; 16]);
    }

    #[test]
    fn random_programs_decode_to_whole_instructions() {
        let mut rng = Rng::new(11);
        let prog = random_program(&mut rng, 40);
        let mut c = crate::core::support::core_with(&prog);
        let (mut pc, end) = (c.pc, c.pc + prog.len() as u32);
        let mut n = 0;
        while pc < end {
            pc = crate::core::uop::lower::decode::decode_at(&mut c, pc).next_pc;
            n += 1;
        }
        assert_eq!((pc, n), (end, 40), "the pool holds complete instructions");
    }

    #[test]
    fn a_program_can_be_drawn_from_a_pool_of_the_callers() {
        let pool: &[&[u16]] = &[&[0x0001], &[0x018e, 0x000a]];
        let mut rng = Rng::new(5);
        let prog = random_program_from(&mut rng, pool, 30);
        assert!(prog.len() >= 30 && prog.len() <= 60);
        assert_eq!(
            prog,
            random_program_from(&mut Rng::new(5), pool, 30),
            "a seed reproduces the program"
        );
    }
}
