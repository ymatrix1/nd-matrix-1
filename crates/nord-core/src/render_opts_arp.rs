//! The arpeggiator switch of the shared firmware options (`--legacy-arp`, D166).
//!
//! The ARM pattern player walks Up / Down / Up-Down over the whole octave pool (the default since
//! D166) and ends an Arp / Poly note at 2/3 of the step (default since D168, `--no-arp-gate` opts
//! out; the ARM never rewrites the pattern flag, `docs/research/arp-gate-flag-trace.md`, OQ32).
//! `walk = false` restores the engine walk.

use crate::engine::Stage4Engine;

/// Selects the firmware arpeggiator walk (`walk`) and the 2/3 gate (`gate`) on the three synth
/// layers. Plain field writes, safe between audio blocks.
pub fn apply_arp_firmware(engine: &mut Stage4Engine, walk: bool, gate: bool) {
    for s in [
        &mut engine.synth_a,
        &mut engine.synth_b,
        &mut engine.synth_c,
    ] {
        s.arp_firmware_walk = walk;
        s.arp_firmware_gate = gate;
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
    fn apply_switches_walk_and_gate_on_all_three_synth_layers() {
        let mut e = Stage4Engine::new(48_000.0);
        assert!(e.synth_a.arp_firmware_walk && e.synth_b.arp_firmware_gate);
        apply_arp_firmware(&mut e, false, false);
        for s in [&e.synth_a, &e.synth_b, &e.synth_c] {
            assert!(!s.arp_firmware_walk && !s.arp_firmware_gate);
        }
        apply_arp_firmware(&mut e, true, true);
        assert!(e.synth_c.arp_firmware_walk && e.synth_c.arp_firmware_gate);
        apply_arp_firmware(&mut e, true, false);
        assert!(e.synth_c.arp_firmware_walk && !e.synth_c.arp_firmware_gate);
    }

    #[test]
    fn firmware_arp_walk_and_gate_are_the_default_and_the_opt_outs_work() {
        assert!(FirmwareOptions::default().arp_firmware);
        assert!(FirmwareOptions::default().arp_firmware_gate);
        assert!(FirmwareOptions::firmware_all().arp_firmware);
        assert!(FirmwareOptions::firmware_all().arp_firmware_gate);
        let n = FirmwareOptions::from_args(&args(&["--no-arp-gate"])).unwrap();
        assert!(n.arp_firmware && !n.arp_firmware_gate && n.summary().contains("no-arp-gate"));
        assert!(!FirmwareOptions::default().summary().contains("no-arp-gate"));
        assert!(!FirmwareOptions::legacy_all().arp_firmware);
        assert!(!FirmwareOptions::legacy_all().arp_firmware_gate);
        let g = FirmwareOptions::from_args(&args(&["--arp-fw-gate"])).unwrap();
        assert!(g.arp_firmware && g.arp_firmware_gate);
        let both = FirmwareOptions::from_args(&args(&["--arp-fw-gate", "--legacy-arp"])).unwrap();
        assert!(
            !both.arp_firmware && !both.arp_firmware_gate,
            "legacy-arp wins"
        );
        assert!(!FirmwareOptions::default().summary().contains("legacy-arp"));
        let o = FirmwareOptions::from_args(&args(&["--legacy-arp"])).unwrap();
        assert!(!o.arp_firmware && o.summary().contains("legacy-arp"));
        // the older spelling stays accepted
        let o = FirmwareOptions::from_args(&args(&["--arp-fw-walk"])).unwrap();
        assert!(o.arp_firmware);
        let mut e = Stage4Engine::new(48_000.0);
        FirmwareOptions::from_args(&args(&["--legacy-arp"]))
            .unwrap()
            .apply_to(&mut e);
        assert!(!e.synth_a.arp_firmware_walk && !e.synth_c.arp_firmware_gate);
        FirmwareOptions::default().apply_to(&mut e);
        assert!(e.synth_a.arp_firmware_walk && e.synth_c.arp_firmware_gate);
        FirmwareOptions::from_args(&args(&["--no-arp-gate"]))
            .unwrap()
            .apply_to(&mut e);
        assert!(e.synth_a.arp_firmware_walk && !e.synth_c.arp_firmware_gate);
    }
}
