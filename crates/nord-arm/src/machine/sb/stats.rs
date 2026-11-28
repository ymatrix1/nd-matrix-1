//! OWNER: S0 (frozen). Counters of the ARM0 execution tiers (gates G0, G2, G3).
//!
//! Plain integers, no allocation: the driver and the executor bump them on the run path. The
//! derived ratios are what `arm-sb-bench` and `arm-sb-ab` print; nothing here is firmware
//! derived (D20, D128).

use super::uop::ExitKind;

/// Tier counters (`SbState::stats`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SbStats {
    /// Traces built / build attempts that failed (a pc is then marked failed).
    pub built: u64,
    pub build_failed: u64,
    /// Whole-cache flushes (uop cap) and traces dropped by a stale page.
    pub flushes: u64,
    pub dropped: u64,
    /// Traces kept after a store to their page (unchanged words).
    pub revalidated: u64,
    /// Trace entries and the guest instructions retired inside traces.
    pub enters: u64,
    pub guest_in_traces: u64,
    /// Guest instructions run by the tier-1 slices between traces.
    pub guest_in_tier1: u64,
    /// Guest instructions executed as `GENERIC` uops.
    pub guest_generic: u64,
    /// Exits taken, by [`ExitKind`].
    pub exits: [u64; ExitKind::COUNT],
    /// Native entries and the exits that left native code for the interpreter (G3: <= 1 %).
    pub native_enters: u64,
    pub native_slow_exits: u64,
    /// Traces rejected by shadow or translation validation (loud by design).
    pub shadow_mismatches: u64,
    pub validations: u64,
}

impl SbStats {
    /// Count one exit of `kind`.
    #[inline]
    pub fn record_exit(&mut self, kind: ExitKind) {
        self.exits[kind as usize] += 1;
    }

    /// Share of guest instructions retired inside traces (G2: coverage >= 85 %).
    pub fn coverage(&self) -> f64 {
        ratio(
            self.guest_in_traces,
            self.guest_in_traces + self.guest_in_tier1,
        )
    }

    /// Share of the trace instructions that ran as `GENERIC` (G2: <= 8 %).
    pub fn generic_share(&self) -> f64 {
        ratio(self.guest_generic, self.guest_in_traces)
    }

    /// Guest instructions per trace entry (G2: >= 15; tier 1 averages 7.8 per block).
    pub fn insns_per_enter(&self) -> f64 {
        ratio(self.guest_in_traces, self.enters)
    }

    /// Share of the tier-1 slice instructions (G2: <= 15 %).
    pub fn slice_share(&self) -> f64 {
        ratio(
            self.guest_in_tier1,
            self.guest_in_traces + self.guest_in_tier1,
        )
    }
}

fn ratio(a: u64, b: u64) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_counter_set_has_zero_ratios_and_no_division_by_zero() {
        let s = SbStats::default();
        assert_eq!(
            (
                s.coverage(),
                s.generic_share(),
                s.insns_per_enter(),
                s.slice_share()
            ),
            (0.0, 0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn ratios_follow_the_counters() {
        let mut s = SbStats {
            enters: 10,
            guest_in_traces: 300,
            guest_in_tier1: 100,
            guest_generic: 30,
            ..SbStats::default()
        };
        assert_eq!(s.coverage(), 0.75);
        assert_eq!(s.slice_share(), 0.25);
        assert_eq!(s.generic_share(), 0.1);
        assert_eq!(s.insns_per_enter(), 30.0);
        s.record_exit(ExitKind::Stale);
        s.record_exit(ExitKind::Stale);
        s.record_exit(ExitKind::Watch);
        assert_eq!(
            (
                s.exits[ExitKind::Stale as usize],
                s.exits[ExitKind::Watch as usize]
            ),
            (2, 1)
        );
    }
}
