//! Deterministic generators for the differential fuzz: a seeded PRNG, edge-biased register
//! values, state fillers, a corpus of valid encodings and random programs.
//!
//! OWNER: W1-VERIFY (generators, fillers and the census corpus), W3-FUZZ (per-family program
//! shapes). No `rand` dependency: the xorshift generator makes a seed reproduce a failure on
//! every platform. One module per concern, everything re-exported here so the paths stay
//! `verify::gen::*`:
//!
//! | Module | Holds |
//! |---|---|
//! | `rng` | [`Rng`], [`EDGE_WORDS`], [`edge_word`] |
//! | `program` | [`core_with_code`], [`CODE_PC`], [`PROGRAM_POOL`], [`random_program`], [`random_program_from`] |
//! | `state` | [`fill_state`] (registers, flags, float MODE1 bits) and the fillers of the hidden state: [`fill_sticky`], [`fill_astat_extra`], [`fill_fifo`], [`fill_stacks`] |
//! | `census` | `census_encodings` (tests only): the valid instruction encodings of the committed census, per kind |
//!
//! A family draws its programs from its own table, or from `census_encodings` filtered to its
//! instruction kinds, through [`random_program_from`]; nobody edits [`PROGRAM_POOL`]. The state
//! of a family's domain (I/M/L/B and the SRAM data window, loop stack, `pending`) is filled in
//! that family's own test module (design P13).
#[cfg(test)]
mod census;
mod program;
mod rng;
mod state;

#[cfg(test)]
pub use census::census_encodings;
pub use program::{core_with_code, random_program, random_program_from, CODE_PC, PROGRAM_POOL};
pub use rng::{edge_word, Rng, EDGE_WORDS};
pub use state::{fill_astat_extra, fill_fifo, fill_stacks, fill_state, fill_sticky};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_api_is_reachable_under_the_old_paths() {
        let mut rng = Rng::new(1);
        let mut c = core_with_code(&random_program(&mut rng, 3));
        fill_state(&mut c, &mut rng, false);
        assert_eq!(c.pc, CODE_PC);
        assert!(PROGRAM_POOL.len() > 10 && EDGE_WORDS.len() == 14);
        assert!(edge_word(&mut rng) != 1);
        assert!(!census_encodings(|_, _| true).is_empty());
    }
}
