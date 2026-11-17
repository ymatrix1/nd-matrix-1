//! The FX rate-match switch of the shared firmware options (`--fx-rate-match`, D137).
//!
//! The firmware Amp drive and cabinet are per-frame stages of the 43,402 Hz DSP frame rate; at the
//! host rate their response is stretched in frequency (`docs/research/brightness-downstream-oracle.md`).
//! `on` runs them behind `LayerFxChain::set_fx_rate_match`. Default off.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_opts::FirmwareOptions;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn switch_reaches_every_fx_chain_and_is_idempotent() {
        let mut e = Stage4Engine::new(48_000.0);
        assert!(e.fx_chains.iter().all(|c| !c.fx_rate_match()));
        apply_fx_rate_match(&mut e, true);
        apply_fx_rate_match(&mut e, true);
        assert!(e.fx_chains.iter().all(|c| c.fx_rate_match()));
        apply_fx_rate_match(&mut e, false);
        assert!(e.fx_chains.iter().all(|c| !c.fx_rate_match()));
    }

    #[test]
    fn flag_is_opt_in_and_survives_apply_exact_toggles() {
        assert!(!FirmwareOptions::default().fx_rate_match);
        assert!(!FirmwareOptions::firmware_all().fx_rate_match);
        assert!(!FirmwareOptions::legacy_all().fx_rate_match);
        let o = FirmwareOptions::from_args(&args(&["--fx-rate-match"])).unwrap();
        assert!(o.fx_rate_match && o.summary().contains("fx-rate-match"));
        let mut e = Stage4Engine::new(48_000.0);
        o.apply_to(&mut e);
        assert!(e.fx_chains.iter().all(|c| c.fx_rate_match()));
        FirmwareOptions::default().apply_exact(&mut e);
        assert!(e.fx_chains.iter().all(|c| !c.fx_rate_match()));
    }
}
