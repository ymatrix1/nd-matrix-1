//! The ARM tick switch of the shared firmware options (`--legacy-1khz-tick`, D131).
//!
//! The measured timer is 24 kHz (`docs/research/arm-tick-timers.md`); the Vox key timeline
//! (`organ/vox_timeline.rs`) and the synth gain de-zipper (`synth/gain_dezip.rs`) use it by
//! default. `on = false` restores the legacy 1 ms tick. The arp gate (OQ32) and PUMP (OQ36) are
//! not touched.

use crate::engine::Stage4Engine;

/// Selects the measured 24 kHz tick (`true`) or the legacy 1 kHz tick (`false`) on both organ
/// layers and the three synth layers. Plain field writes, safe between audio blocks.
pub fn apply_measured_tick(engine: &mut Stage4Engine, on: bool) {
    engine.organ_a.set_measured_tick(on);
    engine.organ_b.set_measured_tick(on);
    for s in [
        &mut engine.synth_a,
        &mut engine.synth_b,
        &mut engine.synth_c,
    ] {
        s.measured_tick = on;
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
    fn switch_reaches_every_organ_and_synth_layer() {
        let mut e = Stage4Engine::new(48_000.0);
        assert!(e.synth_a.measured_tick && e.synth_b.measured_tick && e.synth_c.measured_tick);
        apply_measured_tick(&mut e, false);
        assert!(!e.synth_a.measured_tick && !e.synth_b.measured_tick && !e.synth_c.measured_tick);
        apply_measured_tick(&mut e, true);
        assert!(e.synth_a.measured_tick && e.synth_c.measured_tick);
    }

    #[test]
    fn measured_is_the_default_and_the_legacy_flag_opts_out() {
        assert!(FirmwareOptions::default().measured_tick);
        assert!(FirmwareOptions::firmware_all().measured_tick);
        assert!(!FirmwareOptions::legacy_all().measured_tick);
        let o = FirmwareOptions::from_args(&args(&["--legacy-1khz-tick"])).unwrap();
        assert!(!o.measured_tick && o.summary().contains("legacy-1khz-tick"));
        let o =
            FirmwareOptions::from_args(&args(&["--firmware-all", "--legacy-1khz-tick"])).unwrap();
        assert!(!o.measured_tick);
        let mut e = Stage4Engine::new(48_000.0);
        o.apply_to(&mut e);
        assert!(!e.synth_b.measured_tick);
        FirmwareOptions::default().apply_exact(&mut e);
        assert!(e.synth_b.measured_tick, "apply_exact sets the exact value");
    }
}
