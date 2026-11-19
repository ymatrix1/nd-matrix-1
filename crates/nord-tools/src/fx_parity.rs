//! Null of the engine FX path and of the `FirmwareFx::PARITY` preset against the firmware model
//! (`docs/research/program-ranking-fx.md`). The firmware model of a case is its single port on, the
//! stage that is checked word for word against the SHARC oracle (golden vectors, and the live chain
//! in `tools/fx_vs_oracle.py`, `docs/research/fx-vs-oracle.md`); the PARITY chain has every port
//! of the preset on, so a family that is in the preset must null against that model, while the
//! engine path (no ports) does not. Numbers only.

use crate::fx_cases::{chain, render, stimulus, Case, FS};
use nord_core::cbin::FxDecoded;
use nord_core::engine::Stage4Engine;
use nord_core::fx::{FirmwareFx, LayerFxChain};

/// The chain of a case with the ports `fx` on (every other firmware path as `fx` says).
pub fn chain_with(case: &Case, fx: FirmwareFx) -> LayerFxChain {
    let mut c = LayerFxChain::new(FS);
    c.firmware_fx = fx;
    let dec: &FxDecoded = &case.dec;
    Stage4Engine::apply_fx_chain(&mut c, dec);
    c
}

/// Residual of `a` against the reference `b` in dB (`10 log10(sum (a - b)^2 / sum b^2)`, floored at
/// -300): the null depth, minus infinity for identical signals.
pub fn null_db(a: &[f32], b: &[f32]) -> f64 {
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        num += (f64::from(*x) - f64::from(*y)).powi(2);
        den += f64::from(*y).powi(2);
    }
    10.0 * (num.max(1e-30) / den.max(1e-30)).log10()
}

/// Largest latency difference (frames) the aligned null searches: the firmware stages add 2 to 4
/// frames each (`firmware_fx.rs`), and a chain with more ports on adds the sum.
pub const MAX_LAG: usize = 24;

/// Best [`null_db`] of `a` against `b` over a relative shift of at most [`MAX_LAG`] frames either
/// way (only the overlapping part is compared). A chain with extra ports on is the same model
/// plus a fixed latency, so the null is read at the lag where the two stages line up.
pub fn aligned_null_db(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len().min(b.len());
    let at = |lag: isize| {
        let l = lag.unsigned_abs();
        if lag >= 0 {
            null_db(&a[l..n], &b[..n - l])
        } else {
            null_db(&a[..n - l], &b[l..n])
        }
    };
    (-(MAX_LAG as isize)..=MAX_LAG as isize)
        .map(at)
        .fold(f64::INFINITY, f64::min)
}

/// Null of the engine path and of the PARITY chain against the firmware model of `case` on the
/// stimulus `stim` (`n` frames): `(engine_db, parity_db)`.
pub fn nulls(case: &Case, stim: &str, n: usize) -> (f64, f64) {
    let (x, _) = stimulus(stim, n);
    let model = render(&mut chain(case, true), &x);
    let engine = render(&mut chain(case, false), &x);
    let parity = render(&mut chain_with(case, FirmwareFx::PARITY), &x);
    (
        aligned_null_db(&engine, &model),
        aligned_null_db(&parity, &model),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx_cases::cases;

    fn case(family: &str, label: &str) -> Case {
        cases()
            .into_iter()
            .find(|c| c.family == family && c.label == label)
            .unwrap_or_else(|| panic!("no case {family} {label}"))
    }

    #[test]
    fn null_db_is_minus_infinity_floor_for_equal_and_zero_db_for_silence() {
        let a = [0.5f32, -0.25, 0.125];
        assert!(null_db(&a, &a) < -290.0);
        assert!((null_db(&[0.0; 3], &a)).abs() < 1e-9);
    }

    /// The preset nulls against the firmware model for every Mod 2, Mod 1, EQ and compressor
    /// case that has a port, and the engine path does not (it is a different model).
    #[test]
    fn aligned_null_finds_a_pure_delay() {
        let a: Vec<f32> = (0..200).map(|i| ((i * 37 % 17) as f32) - 8.0).collect();
        let mut b = vec![0.0f32; 5];
        b.extend_from_slice(&a[..195]);
        assert!(null_db(&a, &b) > -10.0);
        assert!(aligned_null_db(&a, &b) < -120.0);
        assert!(aligned_null_db(&b, &a) < -120.0);
    }

    #[test]
    fn parity_nulls_against_the_firmware_model_and_the_engine_does_not() {
        let mut checked = 0;
        for c in cases()
            .iter()
            .filter(|c| ["mod2", "mod1", "eq", "comp"].contains(&c.family))
            .filter(|c| FirmwareFx::PARITY.port(c.port))
        {
            let (engine, parity) = nulls(c, "noise", 4096);
            // Mod 1, Mod 2 and EQ are the same stage with a fixed extra latency; the compressor
            // follows its (delayed) input through a gain computer, so it nulls to float noise only.
            let limit = if c.family == "comp" { -70.0 } else { -120.0 };
            assert!(parity < limit, "{}: parity null {parity:.1} dB", c.label);
            if c.label.ends_with("amt64") || c.label.ends_with("amt127") {
                assert!(engine > -40.0, "{}: engine null {engine:.1} dB", c.label);
            }
            checked += 1;
        }
        assert!(checked >= 40, "only {checked} cases checked");
    }

    #[test]
    fn mod2_chorus_and_eq_treble_are_far_from_the_engine_path() {
        let (e, p) = nulls(&case("mod2", "type4-amt64"), "noise", 8192);
        assert!(e > -20.0 && p < -120.0, "engine {e:.1} parity {p:.1}");
        let (e, p) = nulls(&case("eq", "treble+12"), "notes", 8192);
        assert!(e > -30.0 && p < -120.0, "engine {e:.1} parity {p:.1}");
    }

    /// D163: the default chain runs the firmware Mod 2, so it nulls against the oracle-checked port
    /// for every Mod 2 type and amount, and `--legacy-mod2` (the `LEGACY` ports) does not.
    #[test]
    fn the_default_mod2_nulls_against_the_firmware_port_and_legacy_does_not() {
        let mut n = 0;
        for c in cases().iter().filter(|c| c.family == "mod2") {
            let (x, _) = stimulus("noise", 4096);
            let model = render(&mut chain(c, true), &x);
            let default = render(&mut chain_with(c, FirmwareFx::DEFAULT), &x);
            let legacy = render(&mut chain_with(c, FirmwareFx::LEGACY), &x);
            let d = aligned_null_db(&default, &model);
            assert!(d < -120.0, "{}: default null {d:.1} dB", c.label);
            if c.label.ends_with("amt64") || c.label.ends_with("amt127") {
                let l = aligned_null_db(&legacy, &model);
                assert!(l > -40.0, "{}: legacy null {l:.1} dB", c.label);
            }
            n += 1;
        }
        assert_eq!(n, 36);
    }

    #[test]
    fn the_reverb_ports_are_not_in_the_preset() {
        let reverb = cases().into_iter().find(|c| c.family == "reverb").unwrap();
        assert!(!FirmwareFx::PARITY.port(reverb.port));
        let fx = chain_with(&reverb, FirmwareFx::PARITY).firmware_fx;
        assert!(!fx.reverb && !fx.reverb_mix && fx.mod2 && fx.eq);
    }
}
