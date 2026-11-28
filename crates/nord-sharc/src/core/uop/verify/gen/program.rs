//! Programs: a core with code at a fixed place, a small closed pool of valid encodings and the
//! drawing of random programs from any table. OWNER: W1-VERIFY.
use super::rng::Rng;
use crate::core::Core;

/// Short-word address where [`core_with_code`] places a program (inside the VISA code window,
/// the same place the unit-test helper `support::core_with` uses). VERIFIED: equal to
/// `support::CODE`, checked by `core_with_code_places_the_program_where_the_test_helper_does`.
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

/// Valid encodings (parcels of one instruction each), taken from the AOT lowering test: an
/// immediate load, fixed and float ALU and multiplier forms in 16, 32 and 48 bits, a
/// multifunction, register moves, a bit operation and a NOP. ASSUMED: inherited from the AOT
/// lowering test with no recorded instruction-level source. Plausible because each entry decodes
/// as one whole instruction and executes as one straight-line step
/// (`random_programs_decode_to_whole_instructions`,
/// `every_pool_entry_executes_as_one_straight_line_instruction`); the measured census corpus
/// (`census_encodings`) is the evidence-backed alternative and replaces it for any family test.
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
/// slice of parcels). Every family keeps its own table in its own test module (or filters
/// `census_encodings`) and calls this, so no package edits [`PROGRAM_POOL`] (a shared table every
/// package would append to conflicts at merge). `pool` must not be empty.
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

    #[test]
    fn a_seed_reproduces_the_program_and_other_seeds_differ_and_use_the_whole_pool() {
        let a = random_program(&mut Rng::new(21), 60);
        assert_eq!(a, random_program(&mut Rng::new(21), 60));
        assert_ne!(a, random_program(&mut Rng::new(22), 60));
        let mut rng = Rng::new(3);
        let big = random_program(&mut rng, 400);
        for insn in PROGRAM_POOL {
            assert!(
                big.windows(insn.len()).any(|w| w == *insn),
                "{insn:x?} never drawn"
            );
        }
    }

    #[test]
    fn every_pool_entry_executes_as_one_straight_line_instruction() {
        // the pc moves by the entry's length: no jump, no fault, from the zero and a filled state
        for insn in PROGRAM_POOL {
            for seed in [0u64, 1, 2, 3] {
                let mut c = core_with_code(insn);
                if seed != 0 {
                    crate::core::uop::verify::gen::fill_state(&mut c, &mut Rng::new(seed), false);
                }
                let start = c.pc;
                c.step()
                    .unwrap_or_else(|e| panic!("{insn:x?} (seed {seed}) faulted: {e:?}"));
                assert_eq!(c.pc, start + insn.len() as u32, "{insn:x?} (seed {seed})");
            }
        }
    }
}
