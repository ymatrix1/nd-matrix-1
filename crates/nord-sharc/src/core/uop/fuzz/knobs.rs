//! The environment knobs of the differential fuzz: how many cases, which seed, strict mode.
//!
//! OWNER: W3-FUZZ. The names are the ones of the other fuzz tests of the workspace
//! (`NS4_FUZZ_CASES`, `NS4_FUZZ_SEED`, see `nord-arm/src/machine/sb/testkit.rs`), so one export
//! widens every fuzz at once. A default run is the CI bound (every test of this directory
//! together stays well under 20 s in the debug profile); a long run is
//! `NS4_FUZZ_CASES=2000 cargo test -p nord-sharc --lib uop::fuzz`.
use crate::core::uop::verify::gen::Rng;

/// Parses an unsigned number; a missing or malformed value is `default`.
fn parse_or(value: Option<String>, default: u64) -> u64 {
    value.and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// Reads an unsigned number from the environment.
fn env_u64(name: &str, default: u64) -> u64 {
    parse_or(std::env::var(name).ok(), default)
}

/// Cases of a fuzz test: `NS4_FUZZ_CASES` when set, else `default` (the CI bound of that test).
/// ASSUMED: the per-test defaults (10 to 60 cases) keep the whole directory under the 20 s budget
/// of the debug profile; the measured time is recorded in the commit message of the package.
pub(super) fn cases(default: u64) -> u64 {
    env_u64("NS4_FUZZ_CASES", default)
}

/// The seed offset of a run: `NS4_FUZZ_SEED`, default 0. Case seeds are derived from it, so
/// `NS4_FUZZ_SEED=7` is a different search, and a failing case reproduces from the seed it prints.
pub(super) fn seed() -> u64 {
    env_u64("NS4_FUZZ_SEED", 0)
}

/// Whether a recorded finding fails its test (`NS4_FUZZ_STRICT=1`). A finding is a defect in a
/// file this package does not own: its test runs by default and prints the observation, so the
/// finding is recorded without turning the suite red; strict mode asserts the correct behaviour.
pub(super) fn strict() -> bool {
    env_u64("NS4_FUZZ_STRICT", 0) != 0
}

/// The generator of one test: the same test number and seed offset always draw the same cases.
pub(super) fn rng_for(test: u64) -> Rng {
    Rng::new(0x57A6_E4C0_DE00_0000 ^ test.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_malformed_value_is_the_default() {
        assert_eq!(parse_or(None, 17), 17);
        assert_eq!(parse_or(Some("many".into()), 5), 5);
        assert_eq!(parse_or(Some(" 12 ".into()), 5), 12);
        assert_eq!(parse_or(Some("0".into()), 5), 0);
    }

    #[test]
    fn the_knobs_follow_the_environment_and_default_without_it() {
        if std::env::var_os("NS4_FUZZ_CASES").is_none() {
            assert_eq!(cases(33), 33);
        }
        if std::env::var_os("NS4_FUZZ_SEED").is_none() {
            assert_eq!(seed(), 0);
        }
        if std::env::var_os("NS4_FUZZ_STRICT").is_none() {
            assert!(!strict());
        }
    }

    #[test]
    fn every_test_draws_its_own_stream_and_the_same_one_every_time() {
        let (mut a, mut b) = (rng_for(1), rng_for(2));
        assert_ne!(a.next_u64(), b.next_u64());
        assert_eq!(rng_for(1).next_u64(), rng_for(1).next_u64());
    }
}
