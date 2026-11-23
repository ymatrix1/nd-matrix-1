//! CPAL audio output stream and MIDIR MIDI input listener.

use super::buffer::{buffer_frames_from_args, clamp_buffer, stream_config, with_buffer_fallback};
use super::callback::AudioCallback;
use super::control::ControlLink;
use super::frame::{render_frame_to_f32, render_frame_to_i16};
use super::meter::AudioMeter;
use super::midi_ports::{midi_ports, pick_port, MidiChoice, MidiCtl, MidiPortsWorker};
use super::{standalone_engine, AudioCommand, AudioHandle};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use midir::{Ignore, MidiInput, MidiInputConnection};
use nord_core::engine::TelemetryCell;
use nord_core::midi::MidiEvent;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Spawns the audio output stream (CPAL) and MIDI input listener (MIDIR).
pub fn start_audio_and_midi(active_keys: Arc<[AtomicBool; 128]>) -> AudioHandle {
    let (mut handle, rx) = AudioHandle::standalone();
    let (ports, ports_worker) = midi_ports();
    handle.set_midi_ports(ports);

    // 1. Spawn Audio Output Thread
    let audio_rx = rx;
    let control = handle.control().cloned();
    let to_audio = handle.cmd_sender.clone();
    let telemetry = handle.telemetry().cell();
    let meter = Arc::clone(handle.meter());
    let argv: Vec<String> = std::env::args().collect();
    let buffer = buffer_frames_from_args(&argv);
    let boost = !argv.iter().any(|a| a == "--no-thread-boost");
    std::thread::spawn(move || {
        run_cpal_audio(audio_rx, control, to_audio, telemetry, meter, buffer, boost);
    });

    // 2. Spawn MIDI In Listener
    let midi_handle = handle.clone();
    let midi_keys = Arc::clone(&active_keys);
    std::thread::spawn(move || {
        run_midi_input(midi_handle, midi_keys, ports_worker);
    });

    handle
}

fn run_cpal_audio(
    rx: Receiver<AudioCommand>,
    control: Option<Arc<ControlLink>>,
    to_audio: Sender<AudioCommand>,
    telemetry: Arc<TelemetryCell>,
    meter: Arc<AudioMeter>,
    buffer_frames: Option<u32>,
    boost: bool,
) {
    let bin = super::garbage::spawn_reaper();
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            eprintln!("[Audio] No default audio output device found.");
            return;
        }
    };

    let config = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[Audio] Failed to get default output config: {}", e);
            return;
        }
    };

    let sample_rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;
    meter.set_sample_rate(sample_rate as u32);
    eprintln!(
        "[Audio] {} Hz, {channels} ch, {:?}, device buffer {}",
        sample_rate as u32,
        config.sample_format(),
        match buffer_frames {
            Some(n) => format!("{n} frames (--audio-buffer)"),
            None => format!("driver default (supported {:?})", config.buffer_size()),
        }
    );

    let supported = *config.buffer_size();
    let requested = buffer_frames.map(|n| clamp_buffer(n, &supported));
    if requested != buffer_frames {
        eprintln!(
            "[Audio] --audio-buffer {} is outside the device range {:?}, using {:?}",
            buffer_frames.unwrap_or(0),
            supported,
            requested
        );
    }
    let sample_format = config.sample_format();
    let base_config: cpal::StreamConfig = config.into();
    // The callback owns the engine, so every attempt builds its own (a rejected size drops it).
    let build = |frames: Option<u32>| -> Result<cpal::Stream, String> {
        let mut engine = standalone_engine(sample_rate);
        // From here on the control thread keeps a shadow of this engine and runs every allocating
        // program edit itself (TD-12); the callback below only swaps prepared state in.
        engine.attach_telemetry(Arc::clone(&telemetry));
        if let Some(link) = &control {
            link.attach(sample_rate, to_audio.clone());
        }
        let mut cb = AudioCallback::new(engine, rx.clone(), channels, Arc::clone(&meter))
            .with_thread_boost(boost)
            .with_garbage(bin.clone());
        let err_fn = |err| eprintln!("[Audio] Stream error: {}", err);
        let cfg = stream_config(base_config.clone(), frames);
        match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &cfg,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    cb.run(data, render_frame_to_f32);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &cfg,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    cb.run(data, render_frame_to_i16);
                },
                err_fn,
                None,
            ),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())
    };

    match with_buffer_fallback(requested, build) {
        Ok(s) => match s.play() {
            Ok(()) => loop {
                std::thread::park();
            },
            Err(e) => eprintln!("[Audio] Playback failed: {}", e),
        },
        Err(e) => eprintln!("[Audio] Could not open the output stream: {e}"),
    }
}

/// MIDI-in thread: owns the `midir` connection, follows the GUI's port choice and rescans on
/// request (hot plug). The first scan happens here, so start-up still opens the first port.
fn run_midi_input(
    handle: AudioHandle,
    active_keys: Arc<[AtomicBool; 128]>,
    worker: MidiPortsWorker,
) {
    let mut choice = MidiChoice::First;
    let mut conn: Option<(String, MidiInputConnection<()>)> = None;
    refresh_midi(&handle, &active_keys, &worker, &choice, &mut conn);
    while let Ok(ctl) = worker.rx.recv() {
        if let MidiCtl::Select(c) = ctl {
            choice = c;
        }
        refresh_midi(&handle, &active_keys, &worker, &choice, &mut conn);
    }
}

/// Re-reads the port list, (re)connects to the port `choice` names and publishes the result.
fn refresh_midi(
    handle: &AudioHandle,
    active_keys: &Arc<[AtomicBool; 128]>,
    worker: &MidiPortsWorker,
    choice: &MidiChoice,
    conn: &mut Option<(String, MidiInputConnection<()>)>,
) {
    let mut midi_in = match MidiInput::new("ND Matrix 1 MIDI In") {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[MIDI] Failed to initialize MidiInput: {}", e);
            worker.publish(
                vec![],
                choice.clone(),
                None,
                format!("MIDI unavailable: {e}"),
            );
            return;
        }
    };
    midi_in.ignore(Ignore::None);
    let ports = midi_in.ports();
    let names: Vec<String> = ports
        .iter()
        .map(|p| midi_in.port_name(p).unwrap_or_else(|_| "Unknown".into()))
        .collect();
    let want = pick_port(&names, choice);
    let want_name = want.map(|i| names[i].clone());
    if want_name.is_some() && conn.as_ref().map(|(n, _)| n) == want_name.as_ref() {
        worker.publish(names, choice.clone(), want_name, String::new());
        return;
    }
    if let Some((_, old)) = conn.take() {
        old.close();
        release_stuck_notes(handle, active_keys);
    }
    let (connected, status) = match want {
        Some(i) => {
            println!("[MIDI] Connecting to port: {}", names[i]);
            let (h, k) = (handle.clone(), Arc::clone(active_keys));
            match midi_in.connect(
                &ports[i],
                "nd-matrix-1-input",
                move |_stamp, message, _| handle_midi_bytes(message, &h, &k),
                (),
            ) {
                Ok(c) => {
                    *conn = Some((names[i].clone(), c));
                    (Some(names[i].clone()), String::new())
                }
                Err(e) => {
                    eprintln!("[MIDI] Failed to connect to port: {}", e);
                    (None, format!("Could not open the port: {e}"))
                }
            }
        }
        None if matches!(choice, MidiChoice::Off) => (None, String::new()),
        None if names.is_empty() => (None, "No MIDI input devices found".to_string()),
        None => (None, "Chosen MIDI port not found".to_string()),
    };
    worker.publish(names, choice.clone(), connected, status);
}

/// A port that goes away or is replaced must not leave notes hanging or keys lit.
fn release_stuck_notes(handle: &AudioHandle, active_keys: &[AtomicBool; 128]) {
    for k in active_keys {
        k.store(false, Ordering::Relaxed);
    }
    handle.send_midi(MidiEvent::ControlChange {
        channel: 0,
        controller: nord_core::midi::cc::ALL_NOTES_OFF,
        value: 0,
    });
}

/// Parses raw MIDI bytes and updates the atomic key states and audio engine.
pub fn handle_midi_bytes(message: &[u8], handle: &AudioHandle, active_keys: &[AtomicBool; 128]) {
    if let Some(event) = MidiEvent::from_bytes(message) {
        match event {
            MidiEvent::NoteOn { note, velocity, .. } if (note as usize) < active_keys.len() => {
                active_keys[note as usize].store(velocity > 0, Ordering::Relaxed);
            }
            MidiEvent::NoteOff { note, .. } if (note as usize) < active_keys.len() => {
                active_keys[note as usize].store(false, Ordering::Relaxed);
            }
            _ => {}
        }
        handle.send_midi(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_midi_bytes_note_on_off() {
        let (handle, rx) = AudioHandle::new();
        let active_keys: Arc<[AtomicBool; 128]> =
            Arc::new(std::array::from_fn(|_| AtomicBool::new(false)));

        let note_on = [0x90, 60, 90];
        handle_midi_bytes(&note_on, &handle, &active_keys);
        assert!(active_keys[60].load(Ordering::Relaxed));

        match rx.try_recv() {
            Ok(AudioCommand::Midi(MidiEvent::NoteOn { note, velocity, .. })) => {
                assert_eq!(note, 60);
                assert_eq!(velocity, 90);
            }
            _ => panic!("Expected NoteOn event"),
        }

        let note_off = [0x90, 60, 0];
        handle_midi_bytes(&note_off, &handle, &active_keys);
        assert!(!active_keys[60].load(Ordering::Relaxed));

        match rx.try_recv() {
            Ok(AudioCommand::Midi(MidiEvent::NoteOff { note, .. })) => {
                assert_eq!(note, 60);
            }
            _ => panic!("Expected NoteOff event"),
        }
    }

    #[test]
    fn release_stuck_notes_clears_keys_and_sends_all_notes_off() {
        let (handle, rx) = AudioHandle::new();
        let keys: Arc<[AtomicBool; 128]> = Arc::new(std::array::from_fn(|_| AtomicBool::new(true)));
        release_stuck_notes(&handle, &keys);
        assert!(keys.iter().all(|k| !k.load(Ordering::Relaxed)));
        assert!(matches!(
            rx.try_recv(),
            Ok(AudioCommand::Midi(MidiEvent::ControlChange {
                controller: nord_core::midi::cc::ALL_NOTES_OFF,
                ..
            }))
        ));
    }
}
