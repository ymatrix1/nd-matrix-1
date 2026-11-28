//! Configuration, counters, build report and errors of the uop engine.
//!
//! OWNER: W0 (skeleton); contract changes go back to W0. Every counter is deterministic (a count,
//! never a time), so `tools/uop_gates.py` can evaluate the structural gates without a benchmark.
//! Provenance of the defaults: design section 5, all ASSUMED.

/// Options of [`crate::core::Core::enable_uop`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UopConfig {
    /// Fewest steps a block may have. ASSUMED 1: blocks of one step are legal, the report counts them.
    pub min_block_steps: u32,
    /// Most steps in a block; bounds the checked-loop replay near events. ASSUMED 48, recalibrate
    /// with the `uop-report` block histogram.
    pub max_block_steps: u32,
    /// Shadow-check every n-th block against the interpreter (0 = off; dev only, allocates).
    pub shadow_every: u32,
    /// Use the native tier (Cranelift, feature `uop-native`). Off by default.
    pub native: bool,
    /// `native::FAM_*` bit set of the inline families (0 = call-threaded only).
    pub families: u32,
    /// Translate on the worker thread (`service/`) instead of inline.
    pub async_translate: bool,
    /// Mismatches (or failed validations) after which a block is poisoned. ASSUMED 3.
    pub poison_after: u32,
}

impl Default for UopConfig {
    fn default() -> Self {
        UopConfig {
            min_block_steps: 1,
            max_block_steps: 48,
            shadow_every: 0,
            native: false,
            families: 0,
            async_translate: false,
            poison_after: 3,
        }
    }
}

/// Counters of the engine since `enable_uop` (or the last `uop_reset_stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UopStats {
    pub passes: u64,
    pub blocks_run: u64,
    pub uops: u64,
    /// Steps accounted by uops (of any kind).
    pub steps_uop: u64,
    /// Of those, steps run by `Interp` uops.
    pub steps_interp_uop: u64,
    /// Steps the caller ran through `Core::step` because no block ran.
    pub steps_core_step: u64,
    /// Steps run by native blocks.
    pub steps_native: u64,
    /// Exits by kind, indexed by `Exit as usize`.
    pub exits: [u64; 6],
    /// Failed entry guards, indexed by `GuardFail as usize`.
    pub guard_fail: [u64; 9],
    pub smc: u64,
    pub translations: u64,
    pub poisoned: u64,
    pub maintain_calls: u64,
    pub shadow_checks: u64,
    pub shadow_mismatch: u64,
    pub shadow_skipped: u64,
}

impl UopStats {
    /// The counts accumulated since `earlier` (a snapshot of the same counters); a counter that
    /// went down (a reset in between) reads as 0.
    pub fn since(&self, earlier: &UopStats) -> UopStats {
        let d = |a: u64, b: u64| a.saturating_sub(b);
        let arr = |a: &[u64], b: &[u64]| -> Vec<u64> {
            a.iter().zip(b).map(|(x, y)| d(*x, *y)).collect()
        };
        let mut exits = [0; 6];
        exits.copy_from_slice(&arr(&self.exits, &earlier.exits));
        let mut guard_fail = [0; 9];
        guard_fail.copy_from_slice(&arr(&self.guard_fail, &earlier.guard_fail));
        UopStats {
            passes: d(self.passes, earlier.passes),
            blocks_run: d(self.blocks_run, earlier.blocks_run),
            uops: d(self.uops, earlier.uops),
            steps_uop: d(self.steps_uop, earlier.steps_uop),
            steps_interp_uop: d(self.steps_interp_uop, earlier.steps_interp_uop),
            steps_core_step: d(self.steps_core_step, earlier.steps_core_step),
            steps_native: d(self.steps_native, earlier.steps_native),
            exits,
            guard_fail,
            smc: d(self.smc, earlier.smc),
            translations: d(self.translations, earlier.translations),
            poisoned: d(self.poisoned, earlier.poisoned),
            maintain_calls: d(self.maintain_calls, earlier.maintain_calls),
            shadow_checks: d(self.shadow_checks, earlier.shadow_checks),
            shadow_mismatch: d(self.shadow_mismatch, earlier.shadow_mismatch),
            shadow_skipped: d(self.shadow_skipped, earlier.shadow_skipped),
        }
    }
}

/// What `enable_uop` built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UopBuildReport {
    pub blocks: u32,
    pub uops: u32,
    /// Instructions covered by blocks.
    pub insns: u32,
    /// Of those, instructions lowered as `Interp` uops.
    pub interp_insns: u32,
    /// Code pages holding translated code.
    pub pages: u32,
}

/// Why `enable_uop` or a translation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UopError {
    /// A range is empty, reversed or outside the code window.
    OutOfRange { lo: u32, hi: u32 },
    /// `UopConfig::native` was set but the crate was built without feature `uop-native`.
    NativeUnavailable,
    /// The translator rejected the code.
    Translate(String),
}

impl std::fmt::Display for UopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UopError::OutOfRange { lo, hi } => {
                write!(
                    f,
                    "uop range 0x{lo:x}..0x{hi:x} is empty or outside the code window"
                )
            }
            UopError::NativeUnavailable => {
                write!(
                    f,
                    "native tier requested but built without feature uop-native"
                )
            }
            UopError::Translate(why) => write!(f, "uop translation failed: {why}"),
        }
    }
}

impl std::error::Error for UopError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_design_values_and_everything_is_off() {
        let c = UopConfig::default();
        assert_eq!(
            (c.min_block_steps, c.max_block_steps, c.poison_after),
            (1, 48, 3)
        );
        assert_eq!(
            (c.shadow_every, c.native, c.families, c.async_translate),
            (0, false, 0, false)
        );
    }

    #[test]
    fn since_subtracts_every_counter_and_clamps_a_reset_to_zero() {
        let mut a = UopStats {
            passes: 9,
            uops: 100,
            smc: 2,
            ..UopStats::default()
        };
        a.exits[5] = 7;
        a.guard_fail[8] = 4;
        let mut b = UopStats {
            passes: 4,
            uops: 40,
            ..UopStats::default()
        };
        b.exits[5] = 2;
        let d = a.since(&b);
        assert_eq!(
            (d.passes, d.uops, d.smc, d.exits[5], d.guard_fail[8]),
            (5, 60, 2, 5, 4)
        );
        assert_eq!(b.since(&a), UopStats::default(), "a reset reads as zero");
    }

    #[test]
    fn errors_display_their_cause() {
        let e = UopError::OutOfRange { lo: 4, hi: 2 };
        assert!(e.to_string().contains("0x4..0x2"), "{e}");
        assert!(UopError::NativeUnavailable
            .to_string()
            .contains("uop-native"));
        assert!(UopError::Translate("x".into()).to_string().ends_with(": x"));
    }
}
