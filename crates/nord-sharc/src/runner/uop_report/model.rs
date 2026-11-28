//! The cost model: deterministic counters in, modeled host cycles out (design sections 5 and 9).
//!
//! OWNER: W1-RUN. Every cost is ASSUMED until gate K0 replaces it with a measurement
//! (dispatch 6, transition 38, `Interp` 100 host cycles, design section 5), and the model is never
//! a time: `tools/uop_gates.py` evaluates gate K3 on its output.
//!
//! Provenance: `WORK_PER_STEP` is the mean of the section 9.1 compute and move rows over the
//! 6,155 non-`Interp` steps of the ORG Core2 reference pass: (2,300 x 89 + 2,900 x 18) / 6,155
//! = 41.7, ASSUMED. `NATIVE_PER_STEP` is section 9.2: 236k cycles over 6,280 steps = 37.6,
//! ASSUMED. `BUDGET_CYCLES` is 46.08 us x 4.5 GHz, ASSUMED (the host clock is the dev machine
//! class; confirm with the owner's CPU before reading a cycle count as time).
use crate::core::UopStats;

/// Host cycles per dispatched uop (indirect call, return, `done += steps`). ASSUMED.
pub const DISPATCH: u64 = 6;
/// Host cycles per block transition (exit, link or pc-map probe, entry guard). ASSUMED.
pub const TRANSITION: u64 = 38;
/// Host cycles per `Interp` step (decode lookup plus `exec_cached`), also charged to a step the
/// caller ran through `Core::step`. ASSUMED.
pub const INTERP_STEP: u64 = 100;
/// Host cycles of useful work per non-`Interp` step in Tier 1 (compute and moves mixed). ASSUMED.
pub const WORK_PER_STEP: u64 = 42;
/// Host cycles per step of a native block, dispatch and transitions included. ASSUMED.
pub const NATIVE_PER_STEP: u64 = 38;
/// Host cycles of one real-time pass: 46.08 us x 4.5 GHz = 207,360 (design section 2). ASSUMED.
pub const BUDGET_CYCLES: u64 = 207_360;

/// Which tier a model evaluates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Predecoded uops through the handler table.
    One,
    /// Native blocks; steps not run natively are charged as in Tier 1.
    Two,
}

/// Modeled host cycles for the counters `s` (deterministic, ASSUMED costs above). Saturating, so
/// a corrupt counter cannot panic a debug build.
pub fn model_cycles(s: &UopStats, tier: Tier) -> u64 {
    let interp = s.steps_interp_uop.saturating_add(s.steps_core_step);
    let fast = s.steps_uop.saturating_sub(s.steps_interp_uop);
    let native = match tier {
        Tier::One => 0,
        Tier::Two => s.steps_native.min(fast),
    };
    let tier1 = fast - native;
    // dispatch and block transitions are paid by the Tier 1 share of the fast steps only (the
    // native per-step cost is all-in)
    let share = |x: u64| {
        if fast == 0 {
            x
        } else {
            (u128::from(x) * u128::from(tier1) / u128::from(fast)) as u64
        }
    };
    share(s.uops)
        .saturating_mul(DISPATCH)
        .saturating_add(share(s.blocks_run).saturating_mul(TRANSITION))
        .saturating_add(interp.saturating_mul(INTERP_STEP))
        .saturating_add(tier1.saturating_mul(WORK_PER_STEP))
        .saturating_add(native.saturating_mul(NATIVE_PER_STEP))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ORG Core2 reference pass of design 9.1: 6,280 steps, 8,478 uops, 520 blocks, 125
    /// `Interp` steps.
    fn org_core2() -> UopStats {
        UopStats {
            passes: 1,
            blocks_run: 520,
            uops: 8478,
            steps_uop: 6280,
            steps_interp_uop: 125,
            ..UopStats::default()
        }
    }

    #[test]
    fn tier_one_model_reproduces_the_section_9_total_of_340k_cycles() {
        // 8,478 x 6 + 520 x 38 + 125 x 100 + 6,155 x 42 = 341,638 against the table's 340k
        let c = model_cycles(&org_core2(), Tier::One);
        assert_eq!(c, 341_638);
        assert!(c.abs_diff(340_000) <= 3_000, "{c}");
    }

    #[test]
    fn tier_two_model_reproduces_the_section_9_total_of_236k_cycles_when_every_step_is_native() {
        let s = UopStats {
            steps_native: 6280,
            ..org_core2()
        };
        let c = model_cycles(&s, Tier::Two);
        // 6,155 native steps x 38 + 125 `Interp` steps x 100 = 246,390 against the table's 236k
        assert_eq!(c, 246_390);
        assert!(c.abs_diff(236_000) <= 15_000, "{c}");
        assert!(c < model_cycles(&s, Tier::One), "native is modeled cheaper");
        assert_eq!(
            model_cycles(&s, Tier::One),
            model_cycles(&org_core2(), Tier::One),
            "Tier One ignores the native counter"
        );
    }

    #[test]
    fn a_partly_native_pass_pays_dispatch_and_transitions_for_its_tier_one_share_only() {
        let s = UopStats {
            steps_native: 3000,
            ..org_core2()
        };
        // tier 1 share 3,155 of 6,155 fast steps: uops 4,345 x 6 + blocks 266 x 38 (floored)
        // + 125 x 100 + 3,155 x 42 + 3,000 x 38
        assert_eq!(model_cycles(&s, Tier::Two), 295_188);
        // between the all-Tier-1 and the all-native totals
        assert!(model_cycles(&s, Tier::Two) < 341_638 && model_cycles(&s, Tier::Two) > 246_390);
    }

    #[test]
    fn native_steps_beyond_the_fast_steps_are_clamped_not_double_charged() {
        let s = UopStats {
            steps_native: 1_000_000,
            ..org_core2()
        };
        assert_eq!(model_cycles(&s, Tier::Two), 246_390);
    }

    #[test]
    fn steps_run_by_core_step_are_charged_as_interpreter_steps() {
        let s = UopStats {
            steps_core_step: 10,
            ..UopStats::default()
        };
        assert_eq!(model_cycles(&s, Tier::One), 10 * INTERP_STEP);
        assert_eq!(model_cycles(&UopStats::default(), Tier::Two), 0);
    }

    #[test]
    fn a_run_of_interp_uops_only_pays_all_its_dispatch_and_transitions() {
        let s = UopStats {
            uops: 10,
            blocks_run: 2,
            steps_uop: 10,
            steps_interp_uop: 10,
            ..UopStats::default()
        };
        assert_eq!(
            model_cycles(&s, Tier::One),
            10 * DISPATCH + 2 * TRANSITION + 10 * INTERP_STEP
        );
        assert_eq!(model_cycles(&s, Tier::One), 1_136);
    }

    #[test]
    fn the_tier_one_model_is_linear_in_the_counters() {
        let one = org_core2();
        let two = UopStats {
            passes: 2,
            blocks_run: 1040,
            uops: 16956,
            steps_uop: 12560,
            steps_interp_uop: 250,
            ..UopStats::default()
        };
        assert_eq!(
            model_cycles(&two, Tier::One),
            2 * model_cycles(&one, Tier::One)
        );
    }

    #[test]
    fn corrupt_counters_saturate_instead_of_panicking() {
        let s = UopStats {
            blocks_run: u64::MAX,
            uops: u64::MAX,
            steps_uop: u64::MAX,
            steps_interp_uop: u64::MAX,
            steps_core_step: u64::MAX,
            steps_native: u64::MAX,
            ..UopStats::default()
        };
        assert_eq!(model_cycles(&s, Tier::One), u64::MAX);
        let s = UopStats {
            blocks_run: u64::MAX,
            uops: u64::MAX,
            steps_uop: u64::MAX,
            steps_native: u64::MAX,
            ..UopStats::default()
        };
        assert_eq!(model_cycles(&s, Tier::Two), u64::MAX);
        // more Interp steps than uop steps (a mismatched snapshot) reads as no fast steps
        let s = UopStats {
            steps_uop: 5,
            steps_interp_uop: 9,
            ..UopStats::default()
        };
        assert_eq!(model_cycles(&s, Tier::Two), 9 * INTERP_STEP);
    }

    #[test]
    fn the_budget_is_46_us_at_4_5_ghz() {
        assert_eq!(BUDGET_CYCLES, 207_360);
        assert_eq!((4.608e-5_f64 * 4.5e9).round() as u64, BUDGET_CYCLES);
    }
}
