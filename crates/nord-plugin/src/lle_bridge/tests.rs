use super::fake::FakeBackend;
use super::*;
use nord_core::lle_io::Pedal;

fn on(timing: u32, note: u8) -> NoteEvent<()> {
    NoteEvent::NoteOn {
        timing,
        voice_id: None,
        channel: 0,
        note,
        velocity: 0.8,
    }
}

fn off(timing: u32, note: u8) -> NoteEvent<()> {
    NoteEvent::NoteOff {
        timing,
        voice_id: None,
        channel: 0,
        note,
        velocity: 0.0,
    }
}

fn lle_available() -> LleAvailability {
    LleAvailability::Available
}

fn lle_missing() -> LleAvailability {
    LleAvailability::Unavailable("none".into())
}

#[test]
fn off_by_default_and_for_hle_or_garbage() {
    assert_eq!(LleBridge::default().state(), BridgeState::Off);
    for (env, saved) in [
        (None, None),
        (Some("hle"), Some("lle")),
        (None, Some("hle")),
        (Some("jit"), None),
    ] {
        let b = LleBridge::from_settings_with(env, saved, lle_available);
        assert_eq!(b.state(), BridgeState::Off, "{env:?} {saved:?}");
        assert_eq!(b.status_text(), None);
    }
}

#[test]
fn env_or_saved_lle_without_an_engine_reports_unavailable_and_forwards_nothing() {
    for (env, saved) in [
        (Some("lle"), None),
        (None, Some("lle")),
        (Some("LLE"), Some("hle")),
    ] {
        let mut b = LleBridge::from_settings_with(env, saved, lle_missing);
        assert_eq!(b.state(), BridgeState::Unavailable, "{env:?} {saved:?}");
        assert_eq!(b.status_text(), Some(LLE_UNAVAILABLE));
        assert!(!b.is_active());
        b.forward_host_event(&on(3, 60));
        b.scan_params();
        b.end_block();
        assert_eq!(b.overflow_count(), 0);
        assert!(b.take_engine_ends().is_none());
        assert!(!b.poll_telemetry(&mut TelemetrySnapshot::default()));
    }
}

#[test]
fn the_real_probe_is_unavailable_today() {
    let b = LleBridge::from_settings(Some("lle"), None);
    assert_eq!(b.state(), BridgeState::Unavailable);
}

#[test]
fn available_engine_makes_the_bridge_active() {
    let b = LleBridge::from_settings_with(Some("lle"), None, lle_available);
    assert_eq!(b.state(), BridgeState::Active);
    assert!(b.is_active());
}

#[test]
fn host_events_keep_order_and_sample_offsets() {
    let mut b = LleBridge::active(64);
    let mut ends = b.take_engine_ends().unwrap();
    let block = [
        on(0, 60),
        on(5, 64),
        NoteEvent::MidiCC {
            timing: 5,
            channel: 0,
            cc: 64,
            value: 1.0,
        },
        NoteEvent::MidiPitchBend {
            timing: 100,
            channel: 0,
            value: 0.75,
        },
        NoteEvent::MidiChannelPressure {
            timing: 100,
            channel: 0,
            pressure: 0.5,
        },
        NoteEvent::MidiProgramChange {
            timing: 255,
            channel: 0,
            program: 12,
        },
        off(511, 60),
    ];
    for e in &block {
        b.forward_host_event(e);
    }
    let mut fake = FakeBackend::new();
    assert_eq!(drain_stamped(&mut fake, &mut ends.events), block.len());
    let got: Vec<(u32, LleEvent)> = fake.log().to_vec();
    let offsets: Vec<u32> = got.iter().map(|g| g.0).collect();
    assert_eq!(offsets, [0, 5, 5, 100, 100, 255, 511]);
    assert!(matches!(got[0].1, LleEvent::NoteOn { note: 60, .. }));
    assert!(matches!(got[1].1, LleEvent::NoteOn { note: 64, .. }));
    assert!(matches!(
        got[2].1,
        LleEvent::Pedal {
            pedal: Pedal::Sustain,
            value: 127,
            ..
        }
    ));
    assert!(matches!(got[3].1, LleEvent::PitchBend { .. }));
    assert!(matches!(got[4].1, LleEvent::ChannelAftertouch { .. }));
    assert!(matches!(
        got[5].1,
        LleEvent::ProgramChange { program: 12, .. }
    ));
    assert!(matches!(got[6].1, LleEvent::NoteOff { note: 60, .. }));
}

#[test]
fn wildcard_keys_and_polyphonic_pressure_are_not_forwarded() {
    let mut b = LleBridge::active(8);
    let mut ends = b.take_engine_ends().unwrap();
    b.forward_host_event(&on(0, 255));
    b.forward_host_event(&on(0, 128));
    b.forward_host_event(&NoteEvent::PolyPressure {
        timing: 0,
        voice_id: None,
        channel: 0,
        note: 60,
        pressure: 0.5,
    });
    assert!(ends.events.pop().is_none());
    assert_eq!(b.overflow_count(), 0);
}

#[test]
fn overflow_is_counted_never_blocks_and_triggers_one_resync() {
    let mut b = LleBridge::active(4);
    let mut ends = b.take_engine_ends().unwrap();
    for i in 0..10 {
        b.forward_host_event(&on(i, 60 + i as u8));
    }
    assert_eq!(b.overflow_count(), 6, "4 slots kept, 6 rejected");
    let mut fake = FakeBackend::new();
    assert_eq!(drain_stamped(&mut fake, &mut ends.events), 4);
    assert_eq!(
        fake.log().iter().map(|e| e.0).collect::<Vec<_>>(),
        [0, 1, 2, 3],
        "oldest kept, in order"
    );
    assert_eq!(fake.resyncs(), 1);
    drain_stamped(&mut fake, &mut ends.events);
    assert_eq!(fake.resyncs(), 1);
}

#[test]
fn moved_parameters_become_panel_events_and_unmoved_ones_do_not() {
    let params = Stage4Params::default();
    let mut b = LleBridge::active(64);
    let mut ends = b.take_engine_ends().unwrap();
    b.bind_params(&params);
    b.scan_params();
    assert!(ends.events.pop().is_none(), "baseline is the current value");
    // nih-plug has no public setter outside a host context, so make the stored baseline of the
    // first parameter differ from its live value: the same state as "the host moved it".
    let idx = 0;
    let live = b.shadow[idx];
    b.shadow[idx] = if live < 0.5 { 1.0 } else { 0.0 };
    b.scan_params();
    let got = ends.events.pop().expect("one panel event");
    assert_eq!(got.offset, 0);
    assert!(matches!(
        got.event,
        LleEvent::Panel(nord_core::lle_io::PanelEvent::Analog { id, value }) if id as usize == idx && value == nord_core::lle_io::translate::unit_to_u16(live)
    ));
    assert!(ends.events.pop().is_none());
    b.scan_params();
    assert!(ends.events.pop().is_none(), "unchanged value is not resent");
}

#[test]
fn attached_backend_echoes_events_as_telemetry_to_the_editor_side() {
    let mut b = LleBridge::active(64);
    assert!(b.attach_backend(Box::new(FakeBackend::new())));
    assert!(
        !b.attach_backend(Box::new(FakeBackend::new())),
        "ends already taken"
    );
    let reader = b.telemetry_reader().unwrap();
    b.forward_host_event(&on(42, 61));
    b.end_block();
    let mut snap = TelemetrySnapshot::default();
    assert!(b.poll_telemetry(&mut snap));
    assert_eq!(snap.frame, 1);
    assert!(snap.led(61));
    assert_eq!(&snap.oled[1][..4], &42u32.to_le_bytes());
    b.forward_host_event(&off(7, 61));
    b.end_block();
    let mut again = TelemetrySnapshot::default();
    assert!(reader.read(&mut again));
    assert_eq!(again.frame, 2);
    assert!(!again.led(61));
}

#[test]
fn attach_needs_an_active_bridge() {
    let mut b = LleBridge::off();
    assert!(!b.attach_backend(Box::new(FakeBackend::new())));
    b.bind_params(&Stage4Params::default());
    assert!(b.watched.is_empty(), "bind is a no-op when off");
}
