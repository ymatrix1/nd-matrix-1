//! Flags and controller messages of `program_sweep` (D166): `--legacy-arp`, `--legacy-morph`,
//! `--arp-fw-gate`, `--legacy-fx-rate` (D180), `--legacy-synth-resonance` (D223), and the Mod Wheel / Control Pedal / aftertouch values sent before the notes.

use nord_core::midi::MidiEvent;
use nord_core::render::TimedEvent;
use nord_core::render_opts::FirmwareOptions;

/// Controller messages sent before the notes: Mod Wheel (CC 1), the Control Pedal (CC 11) and
/// channel aftertouch, each only when its value is given.
pub fn controller_events(
    wheel: Option<u8>,
    aftertouch: Option<u8>,
    pedal: Option<u8>,
) -> Vec<TimedEvent> {
    let cc = |controller, value: u8| TimedEvent {
        frame: 0,
        event: MidiEvent::ControlChange {
            channel: 0,
            controller,
            value: value.min(127),
        },
    };
    let mut v = Vec::new();
    v.extend(wheel.map(|x| cc(1, x)));
    v.extend(pedal.map(|x| cc(11, x)));
    v.extend(aftertouch.map(|x| TimedEvent {
        frame: 0,
        event: MidiEvent::ChannelAftertouch {
            channel: 0,
            pressure: x.min(127),
        },
    }));
    v
}

/// `--legacy-arp` / `--legacy-morph` restore the pre-D166 arpeggiator walk and morph law;
/// `--no-arp-gate` turns the 2/3 gate (default since D168) off; `--legacy-arp` turns it off too.
pub fn apply_legacy_flags(o: &mut FirmwareOptions, args: &[String]) {
    let has = |name: &str| args.iter().any(|a| a == name);
    if has("--arp-fw-gate") {
        o.arp_firmware_gate = true;
    }
    if has("--no-arp-gate") {
        o.arp_firmware_gate = false;
    }
    if has("--legacy-arp") {
        o.arp_firmware = false;
        o.arp_firmware_gate = false;
    }
    if has("--legacy-morph") {
        o.firmware_morph = false;
    }
    // D180: the rate match is the default in both modes; `--legacy-fx-rate` turns it off in both.
    if has("--legacy-fx-rate") {
        o.fx_rate_match = false;
    }
    // D223: the firmware resonance position is the default in both modes; the legacy flag turns
    // it off in both.
    if has("--legacy-synth-resonance") {
        o.synth_fw_resonance = false;
    }
    // D173: the default FX ports are PARITY; the firmware mode (every port on) is left alone.
    if has("--legacy-fx-ports") && o.fx == nord_core::fx::FirmwareFx::DEFAULT {
        o.fx = nord_core::fx::FirmwareFx::PRE_PARITY;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn legacy_flags_turn_the_arp_and_morph_defaults_off_in_both_modes() {
        let mut o = FirmwareOptions::default();
        apply_legacy_flags(&mut o, &args(&[]));
        assert!(o.arp_firmware && o.firmware_morph && o.arp_firmware_gate);
        apply_legacy_flags(&mut o, &args(&["--legacy-arp"]));
        assert!(!o.arp_firmware && o.firmware_morph);
        apply_legacy_flags(&mut o, &args(&["--legacy-morph"]));
        assert!(!o.firmware_morph);
    }

    #[test]
    fn legacy_fx_ports_restores_the_d163_ports_in_the_default_mode_only() {
        use nord_core::fx::FirmwareFx;
        let mut o = FirmwareOptions::default();
        apply_legacy_flags(&mut o, &args(&[]));
        assert_eq!(o.fx, FirmwareFx::PARITY);
        apply_legacy_flags(&mut o, &args(&["--legacy-fx-ports"]));
        assert_eq!(o.fx, FirmwareFx::PRE_PARITY);
        let mut fw = FirmwareOptions::firmware_all();
        apply_legacy_flags(&mut fw, &args(&["--legacy-fx-ports"]));
        assert_eq!(fw.fx, FirmwareFx::ALL);
    }

    #[test]
    fn legacy_synth_resonance_turns_the_d223_default_off_in_both_modes() {
        for mut o in [FirmwareOptions::default(), FirmwareOptions::firmware_all()] {
            apply_legacy_flags(&mut o, &args(&[]));
            assert!(o.synth_fw_resonance);
            apply_legacy_flags(&mut o, &args(&["--legacy-synth-resonance"]));
            assert!(!o.synth_fw_resonance);
        }
    }

    #[test]
    fn legacy_fx_rate_turns_the_rate_match_off_in_both_modes_and_is_on_by_default() {
        for mut o in [FirmwareOptions::default(), FirmwareOptions::firmware_all()] {
            apply_legacy_flags(&mut o, &args(&[]));
            assert!(o.fx_rate_match);
            apply_legacy_flags(&mut o, &args(&["--legacy-fx-rate"]));
            assert!(!o.fx_rate_match);
        }
    }

    #[test]
    fn the_gate_is_default_no_arp_gate_opts_out_and_legacy_arp_wins() {
        let mut o = FirmwareOptions::default();
        apply_legacy_flags(&mut o, &args(&["--no-arp-gate"]));
        assert!(o.arp_firmware && !o.arp_firmware_gate);
        let mut o = FirmwareOptions::default();
        apply_legacy_flags(&mut o, &args(&["--arp-fw-gate"]));
        assert!(o.arp_firmware && o.arp_firmware_gate);
        apply_legacy_flags(&mut o, &args(&["--arp-fw-gate", "--legacy-arp"]));
        assert!(!o.arp_firmware && !o.arp_firmware_gate);
    }

    #[test]
    fn controller_events_send_only_the_given_controllers() {
        assert!(controller_events(None, None, None).is_empty());
        let ev = controller_events(Some(127), Some(64), Some(200));
        assert_eq!(ev.len(), 3);
        assert!(ev.iter().all(|e| e.frame == 0));
        assert!(matches!(
            ev[0].event,
            MidiEvent::ControlChange {
                controller: 1,
                value: 127,
                ..
            }
        ));
        assert!(matches!(
            ev[1].event,
            MidiEvent::ControlChange {
                controller: 11,
                value: 127,
                ..
            }
        ));
        assert!(matches!(
            ev[2].event,
            MidiEvent::ChannelAftertouch { pressure: 64, .. }
        ));
    }
}
