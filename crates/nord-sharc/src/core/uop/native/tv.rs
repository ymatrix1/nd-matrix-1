//! Translation validation of native blocks against Tier 1.
//!
//! OWNER: W4-NSVC. On the worker, every new native block runs on [`STATES`] random states
//! against the Tier 1 result and `exact_diff` must be empty before install; a block that fails
//! is poisoned and keeps running in Tier 1 (design 6.5). Skeleton: the acceptance rule only.
use crate::core::uop::config::UopConfig;

/// Random states per native block. ASSUMED 64 (K in design 6.5); raise it if the fuzz finds a
/// native bug the validation missed.
pub const STATES: u32 = 64;

/// Whether a block with `mismatches` failed states may be installed.
pub fn accepts(mismatches: u32) -> bool {
    mismatches == 0
}

/// Failed validations after which a block is poisoned (`UopConfig::poison_after`).
pub fn poisons(cfg: &UopConfig, failures: u32) -> bool {
    failures >= cfg.poison_after
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_clean_validation_installs_and_repeated_failures_poison() {
        assert!(accepts(0) && !accepts(1));
        let cfg = UopConfig::default();
        assert!(!poisons(&cfg, cfg.poison_after - 1) && poisons(&cfg, cfg.poison_after));
        assert_eq!(STATES, 64);
    }
}
