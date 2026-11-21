//! The FX rate-match switch of the shared firmware options (`--fx-rate-match`, D137).
//!
//! The firmware Amp drive and cabinet are per-frame stages of the 43,402 Hz DSP frame rate; at the
//! host rate their response is stretched in frequency (`docs/research/brightness-downstream-oracle.md`).
//! `on` runs them behind `LayerFxChain::set_fx_rate_match`. Default on since D176 (owner approval of
//! 2026-11-21); `--legacy-fx-rate` turns it off. Host rates the converter does not support
//! (`RateMatch::supports`) stay on the per-host-frame path.

use crate::engine::Stage4Engine;

/// Switches the rate match on or off on all seven FX chains. Allocates the kernel tables when
/// switched on (only when the state changes), so call it between audio blocks.
pub fn apply_fx_rate_match(engine: &mut Stage4Engine, on: bool) {
    for c in engine.fx_chains.iter_mut() {
        if c.fx_rate_match() != on {
            c.set_fx_rate_match(on);
        }
    }
}

/// Switches the idle skip on or off on all seven FX chains (D161). Plain field writes, so it is
/// safe between audio blocks; a changed state restarts each rack's quiet count.
pub fn apply_fx_idle_skip(engine: &mut Stage4Engine, on: bool) {
    for c in engine.fx_chains.iter_mut() {
        if c.idle_skip() != on {
            c.set_idle_skip(on);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_opts::FirmwareOptions;

    #[test]
    fn idle_skip_flag_is_opt_in_parses_describes_and_applies_exactly() {
        assert!(!FirmwareOptions::default().fx_idle_skip);
        assert!(!FirmwareOptions::firmware_all().fx_idle_skip);
        assert!(!FirmwareOptions::legacy_all().fx_idle_skip);
        let o = FirmwareOptions::from_args(&["--fx-idle-skip".to_string()]).unwrap();
        assert!(o.fx_idle_skip && o.summary().contains("fx-idle-skip"));
        let mut e = Stage4Engine::new(48_000.0);
        o.apply_to(&mut e);
        assert!(e.fx_chains.iter().all(|c| c.idle_skip()));
        FirmwareOptions::default().apply_exact(&mut e);
        assert!(e.fx_chains.iter().all(|c| !c.idle_skip()));
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn switch_reaches_every_fx_chain_and_is_idempotent() {
        let mut e = Stage4Engine::new(48_000.0);
        assert!(e.fx_chains.iter().all(|c| c.fx_rate_match()), "default on");
        apply_fx_rate_match(&mut e, false);
        apply_fx_rate_match(&mut e, false);
        assert!(e.fx_chains.iter().all(|c| !c.fx_rate_match()));
        apply_fx_rate_match(&mut e, true);
        apply_fx_rate_match(&mut e, true);
        assert!(e.fx_chains.iter().all(|c| c.fx_rate_match()));
    }

    #[test]
    fn unsupported_host_rates_stay_on_the_host_frame_path() {
        let mut e = Stage4Engine::new(192_000.0);
        assert!(e.fx_chains.iter().all(|c| !c.fx_rate_match()));
        apply_fx_rate_match(&mut e, true);
        assert!(e.fx_chains.iter().all(|c| !c.fx_rate_match()));
        let e = Stage4Engine::new(22_050.0);
        assert!(e.fx_chains.iter().all(|c| !c.fx_rate_match()));
    }

    #[test]
    fn flag_is_default_on_and_legacy_fx_rate_opts_out_through_apply_exact() {
        assert!(FirmwareOptions::default().fx_rate_match);
        assert!(FirmwareOptions::firmware_all().fx_rate_match);
        assert!(!FirmwareOptions::legacy_all().fx_rate_match);
        let d = FirmwareOptions::default();
        assert!(!d.summary().contains("rate"), "{}", d.summary());
        // `--fx-rate-match` stays accepted and is a no-op on the default.
        let o = FirmwareOptions::from_args(&args(&["--fx-rate-match"])).unwrap();
        assert_eq!(o, d);
        let l = FirmwareOptions::from_args(&args(&["--legacy-fx-rate"])).unwrap();
        assert!(!l.fx_rate_match && l.summary().contains("legacy-fx-rate"));
        // The opt-out wins over the old opt-in flag.
        let w =
            FirmwareOptions::from_args(&args(&["--fx-rate-match", "--legacy-fx-rate"])).unwrap();
        assert!(!w.fx_rate_match);
        let mut e = Stage4Engine::new(48_000.0);
        l.apply_to(&mut e);
        assert!(e.fx_chains.iter().all(|c| !c.fx_rate_match()));
        d.apply_exact(&mut e);
        assert!(e.fx_chains.iter().all(|c| c.fx_rate_match()));
        l.apply_exact(&mut e);
        assert!(e.fx_chains.iter().all(|c| !c.fx_rate_match()));
    }
}
