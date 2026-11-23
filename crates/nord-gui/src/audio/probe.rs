//! `nd-matrix-1 --audio-probe <secs>`: the real cpal output callback without a window.
//!
//! `ns4 bench --gui` times the render on a plain thread, so it cannot see what the driver and the
//! scheduler do to the real callback thread. The probe opens the default output device exactly like
//! the GUI (same `AudioCallback`, buffer flags, thread boost), plays the bench engine (a factory
//! program from `--backup` / `--program`, `--arp`, `--keys` held notes) for `<secs>` seconds and prints
//! the meter: late blocks, worst callback, slow callback gaps and the thread priority. It makes sound.
//!
//! `nd-matrix-1 --audio-probe 20 --backup <.ns4b> --program "Horizon Whl" --arp up --firmware-fx
//! --keys 6 --audio-buffer 480 [--no-thread-boost]`

use super::buffer::{buffer_frames_from_args, clamp_buffer, stream_config, with_buffer_fallback};
use super::callback::AudioCallback;
use super::frame::{render_frame_to_f32, render_frame_to_i16};
use super::meter::AudioMeter;
use super::standalone_engine;
use super::AudioCommand;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use nord_core::bench_gui::GuiBenchConfig;
use nord_core::midi::MidiEvent;
use nord_core::render_opts::FirmwareOptions;
use std::sync::Arc;
use std::time::Duration;

/// Seconds of run time and the number after `flag`, if present.
fn value<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    args.iter()
        .position(|a| a == flag)?
        .checked_add(1)
        .and_then(|i| args.get(i))?
        .parse()
        .ok()
}

/// `Some(seconds)` when the arguments ask for the probe.
pub fn requested(args: &[String]) -> Option<f32> {
    value::<f32>(args, "--audio-probe").filter(|s| *s > 0.0)
}

/// Runs the probe and returns the report. Blocks for the run time.
pub fn run(args: &[String]) -> Result<String, String> {
    let secs = requested(args).ok_or("--audio-probe needs a number of seconds")?;
    let options = FirmwareOptions::from_args(args)?;
    let keys: u8 = value(args, "--keys").unwrap_or(6);
    let layers: u8 = value(args, "--layers").unwrap_or(3);
    let boost = !args.iter().any(|a| a == "--no-thread-boost");
    let source = nord_core::bench_gui_prog::parse(args)?;
    let cfg = GuiBenchConfig {
        layers,
        keys,
        options,
        ..GuiBenchConfig::default()
    };

    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no output device")?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("output config: {e}"))?;
    let rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;
    let format = config.sample_format();
    let requested_buffer =
        buffer_frames_from_args(args).map(|n| clamp_buffer(n, config.buffer_size()));
    let base: cpal::StreamConfig = config.into();

    let meter = Arc::new(AudioMeter::default());
    meter.set_sample_rate(rate as u32);
    let (tx, rx) = crossbeam_channel::bounded::<AudioCommand>(1024);
    let bin = super::garbage::spawn_reaper();
    let build = |frames: Option<u32>| -> Result<cpal::Stream, String> {
        let mut engine = standalone_engine(rate);
        cfg.options.apply_exact(&mut engine);
        if let Some(src) = &source {
            engine.scene.scene_2_active = cfg.layers > 3;
            src.apply(&cfg, &mut engine)?;
        }
        let mut cb = AudioCallback::new(engine, rx.clone(), channels, Arc::clone(&meter))
            .with_thread_boost(boost)
            .with_garbage(bin.clone());
        let stream_cfg = stream_config(base.clone(), frames);
        let err = |e| eprintln!("[probe] stream error: {e}");
        match format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &stream_cfg,
                move |d: &mut [f32], _: &cpal::OutputCallbackInfo| cb.run(d, render_frame_to_f32),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &stream_cfg,
                move |d: &mut [i16], _: &cpal::OutputCallbackInfo| cb.run(d, render_frame_to_i16),
                err,
                None,
            ),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())
    };
    let stream = with_buffer_fallback(requested_buffer, build)?;
    stream.play().map_err(|e| e.to_string())?;
    for k in 0..keys {
        let _ = tx.send(AudioCommand::Midi(MidiEvent::NoteOn {
            channel: 0,
            note: 36 + k,
            velocity: 100,
        }));
    }
    // Two seconds of warm-up (caches, the one-time boost, voice start) stay out of the figures.
    std::thread::sleep(Duration::from_secs(2));
    meter.request_reset();
    std::thread::sleep(Duration::from_secs_f32(secs));
    drop(stream);
    let s = meter.snapshot();
    Ok(format!(
        "[probe] {rate} Hz, {channels} ch, {format:?}, buffer {}, {secs} s, program {}, {}\n[probe] {}",
        requested_buffer.map_or("driver default".to_string(), |n| format!("{n} frames")),
        source.as_ref().map_or("none", |s| s.entry.as_str()),
        cfg.options.summary(),
        s.summary()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_probe_is_requested_by_a_positive_number_of_seconds() {
        assert_eq!(requested(&a(&["x"])), None);
        assert_eq!(requested(&a(&["x", "--audio-probe", "12.5"])), Some(12.5));
        assert_eq!(requested(&a(&["x", "--audio-probe", "0"])), None);
        assert_eq!(requested(&a(&["x", "--audio-probe", "soon"])), None);
        assert_eq!(requested(&a(&["x", "--audio-probe"])), None);
    }

    #[test]
    fn values_are_read_after_their_flag() {
        let args = a(&["x", "--keys", "9", "--layers", "7"]);
        assert_eq!(value::<u8>(&args, "--keys"), Some(9));
        assert_eq!(value::<u8>(&args, "--layers"), Some(7));
        assert_eq!(value::<u8>(&args, "--arp"), None);
    }
}
