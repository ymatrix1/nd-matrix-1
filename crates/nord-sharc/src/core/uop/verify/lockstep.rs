//! Lockstep of the interpreter and the uop engine from one state, compared every chunk.
//!
//! OWNER: W3-FUZZ. Modeled on `runner/jit_diff.rs`. [`lockstep_cores`] is complete and works on
//! any two cores in the same state (synthetic or real). The real-firmware entry
//! [`firmware_lockstep`] needs the user's own firmware (D128: none in git), so it checks the
//! environment variable [`FIRMWARE_ENV`] and reports `Skipped` instead of failing (never
//! `#[ignore]`). Skeleton: it always reports `Skipped`; the package adds the pass loader.
use super::compare::{exact_diff, ExactDiff};
use crate::core::Core;

/// Environment variable naming the local firmware directory for real-trace lockstep.
pub const FIRMWARE_ENV: &str = "NORD_UOP_FW";

/// Result of a lockstep run that did not find a difference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing ran, and why.
    Skipped(&'static str),
    /// `steps` interpreter steps were compared in chunks.
    Done { steps: u64 },
}

/// Runs `a` with `Core::run` and `b` with `Core::run_uop` for `total` steps in chunks of `chunk`
/// (at least 1) and compares after every chunk; stops after a chunk in which they faulted.
/// Returns the steps compared, or the first difference (a differing fault is reported as the
/// field `fault`).
pub fn lockstep_cores(
    a: &mut Core,
    b: &mut Core,
    total: u64,
    chunk: u64,
) -> Result<u64, ExactDiff> {
    let chunk = chunk.max(1);
    let mut done = 0;
    while done < total {
        let n = chunk.min(total - done);
        let (fa, fb) = (a.run(n), b.run_uop(n));
        if fa != fb {
            return Err(ExactDiff {
                field: "fault",
                a: format!("{fa:?}"),
                b: format!("{fb:?}"),
            });
        }
        if let Some(d) = exact_diff(a, b) {
            return Err(d);
        }
        done += n;
        if fa.is_some() {
            break;
        }
    }
    Ok(done)
}

/// Real-trace lockstep on the local firmware passes (ORG, PNO, FX, SY1SY2, SMP).
pub fn firmware_lockstep(chunk: u64) -> Result<Outcome, ExactDiff> {
    let _ = chunk;
    Ok(Outcome::Skipped(
        if std::env::var_os(FIRMWARE_ENV).is_none() {
            "NORD_UOP_FW is not set"
        } else {
            "the firmware pass loader is not written yet"
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::verify::gen::{core_with_code, fill_state, random_program, Rng, CODE_PC};

    #[test]
    fn two_cores_in_one_state_stay_equal_chunk_after_chunk() {
        let mut rng = Rng::new(5);
        let program = random_program(&mut rng, 30);
        let (mut a, mut b) = (core_with_code(&program), core_with_code(&program));
        fill_state(&mut a, &mut Rng::new(8), true);
        fill_state(&mut b, &mut Rng::new(8), true);
        b.enable_uop(
            UopConfig::default(),
            &[(CODE_PC, CODE_PC + program.len() as u32)],
        )
        .unwrap();
        assert_eq!(lockstep_cores(&mut a, &mut b, 30, 7), Ok(30));
    }

    #[test]
    fn a_perturbed_core_is_caught_at_the_first_chunk() {
        let (mut a, mut b) = (core_with_code(&[0x0001; 8]), core_with_code(&[0x0001; 8]));
        b.r[2] ^= 1;
        assert_eq!(lockstep_cores(&mut a, &mut b, 8, 4).unwrap_err().field, "r");
    }

    #[test]
    fn the_firmware_entry_skips_cleanly_without_firmware() {
        assert!(matches!(firmware_lockstep(64), Ok(Outcome::Skipped(_))));
    }
}
