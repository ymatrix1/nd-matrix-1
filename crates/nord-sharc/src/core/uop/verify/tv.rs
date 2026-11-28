//! Tier 1 translation validation: the engine against the interpreter on random states.
//!
//! OWNER: W3-FUZZ. At `enable_uop` every new block runs on 8 random states (ASSUMED) off the
//! audio path; a block that differs is poisoned (design 6.5). [`validate`] is the engine-level
//! form: it runs the armed engine from the current pc on `states` random states through
//! [`super::shadow::shadow_run`], so the check is the same one shadow mode uses. Skeleton:
//! complete; the per-block poisoning loop in `enable_uop` is W3-FUZZ.
use super::compare::ExactDiff;
use super::gen::{fill_state, Rng};
use super::shadow::{shadow_run, Shadow};
use super::snap::CoreSnap;
use crate::core::Core;

/// Runs the engine for `steps` steps on each of `states` random states (seeded) and checks every
/// run against the interpreter. The core is a scratch core: its registers end up random. Returns
/// the number of runs that were compared (runs on cores with peripherals are not).
pub fn validate(core: &mut Core, states: u32, seed: u64, steps: u64) -> Result<u32, ExactDiff> {
    let start = CoreSnap::capture(core);
    let simd = core.simd();
    let mut rng = Rng::new(seed);
    let mut compared = 0;
    for _ in 0..states {
        start.restore(core);
        fill_state(core, &mut rng, simd);
        match shadow_run(core, |c| {
            let _ = c.run_uop(steps);
        }) {
            Shadow::Match => compared += 1,
            Shadow::Mismatch(d) => return Err(d),
            Shadow::Skipped => {}
        }
    }
    start.restore(core);
    Ok(compared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::verify::gen::{core_with_code, random_program, CODE_PC};

    #[test]
    fn the_engine_validates_on_every_random_state_and_the_start_state_is_restored() {
        let program = random_program(&mut Rng::new(21), 8);
        let mut c = core_with_code(&program);
        c.enable_uop(
            UopConfig::default(),
            &[(CODE_PC, CODE_PC + program.len() as u32)],
        )
        .unwrap();
        assert_eq!(validate(&mut c, 8, 77, 8), Ok(8));
        assert_eq!((c.pc, c.steps, c.r), (CODE_PC, 0, [0; 16]));
    }

    #[test]
    fn zero_states_validate_nothing() {
        let mut c = core_with_code(&[0x0001]);
        assert_eq!(validate(&mut c, 0, 1, 1), Ok(0));
    }
}
