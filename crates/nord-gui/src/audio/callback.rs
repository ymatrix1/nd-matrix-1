//! The body of the CPAL output callback, free of `cpal` types so it can be tested and metered.
//!
//! Per callback: drain the command queue, render every frame, publish telemetry, then record the
//! timings and the output level in the [`AudioMeter`]. No allocation, lock or I/O here
//! (`.agents/rules/dsp-and-realtime.md`); `Instant::now` is a clock read, not a syscall that blocks.

use super::meter::{AudioMeter, BlockSample, BlockStats};
use super::{apply_audio_command, AudioCommand};
use crossbeam_channel::Receiver;
use nord_core::bench::STANDALONE_MASTER_VOL;
use nord_core::engine::Stage4Engine;
use std::sync::Arc;
use std::time::Instant;

/// State the audio thread owns for the life of the stream.
pub struct AudioCallback {
    engine: Stage4Engine,
    rx: Receiver<AudioCommand>,
    channels: usize,
    master_vol: f32,
    meter: Arc<AudioMeter>,
}

impl AudioCallback {
    pub fn new(
        engine: Stage4Engine,
        rx: Receiver<AudioCommand>,
        channels: usize,
        meter: Arc<AudioMeter>,
    ) -> Self {
        Self {
            engine,
            rx,
            channels: channels.max(1),
            master_vol: STANDALONE_MASTER_VOL,
            meter,
        }
    }

    /// Fills `data` (interleaved, `channels` wide) and records the block in the meter.
    /// `write` stores one stereo frame in the device sample format.
    pub fn run<S>(&mut self, data: &mut [S], write: fn(usize, f32, f32, &mut [S])) {
        let t0 = Instant::now();
        while let Ok(cmd) = self.rx.try_recv() {
            apply_realtime(cmd, &mut self.engine, &mut self.master_vol);
        }
        let t1 = Instant::now();
        let mut stats = BlockStats::default();
        let vol = self.master_vol;
        for frame in data.chunks_mut(self.channels) {
            let [l, r] = self.engine.render_frame();
            let (l, r) = (l * vol, r * vol);
            stats.push(l, r);
            write(self.channels, l, r, frame);
        }
        let t2 = Instant::now();
        self.engine.publish_telemetry();
        let t3 = Instant::now();
        let ns = |a: Instant, b: Instant| b.duration_since(a).as_nanos() as u64;
        self.meter.record(BlockSample {
            frames: (data.len() / self.channels) as u32,
            total_ns: ns(t0, t3),
            drain_ns: ns(t0, t1),
            publish_ns: ns(t2, t3),
            peak: stats.peak,
            over_0db: stats.over_0db,
        });
    }
}

/// Applies one command inside the audio callback. A control operation (allocating) that reached
/// this thread despite the control link is dropped and counted, never executed here.
pub fn apply_realtime(cmd: AudioCommand, engine: &mut Stage4Engine, master_vol: &mut f32) {
    if matches!(&cmd, AudioCommand::Program(e) if e.is_control_op()) {
        engine.note_retire_overflow();
        std::mem::forget(cmd);
        return;
    }
    apply_audio_command(cmd, engine, master_vol);
}

#[cfg(test)]
mod tests {
    use super::super::frame::{render_frame_to_f32, render_frame_to_i16};
    use super::super::standalone_engine;
    use super::*;
    use nord_core::midi::MidiEvent;

    fn callback(channels: usize) -> (AudioCallback, crossbeam_channel::Sender<AudioCommand>) {
        let (tx, rx) = crossbeam_channel::bounded(16);
        let meter = Arc::new(AudioMeter::default());
        meter.set_sample_rate(48_000);
        (
            AudioCallback::new(standalone_engine(48_000.0), rx, channels, meter),
            tx,
        )
    }

    #[test]
    fn a_callback_records_one_block_with_its_frame_count() {
        let (mut cb, _tx) = callback(2);
        let mut data = [0.0f32; 256];
        cb.run(&mut data, render_frame_to_f32);
        let s = cb.meter.snapshot();
        assert_eq!((s.blocks, s.frames), (1, 128));
        assert!(s.worst_ms > 0.0);
    }

    #[test]
    fn drained_midi_sounds_and_the_peak_is_metered() {
        let (mut cb, tx) = callback(2);
        let note = |n| {
            AudioCommand::Midi(MidiEvent::NoteOn {
                channel: 0,
                note: n,
                velocity: 120,
            })
        };
        for n in [48, 55, 60, 64, 67] {
            tx.send(note(n)).unwrap();
        }
        let mut data = [0i16; 2 * 4800];
        cb.run(&mut data, render_frame_to_i16);
        assert!(data.iter().any(|&s| s != 0), "the chord is audible");
        let s = cb.meter.snapshot();
        assert!(s.peak > 0.0 && s.peak.is_finite());
        assert!(s.worst_drain_ms > 0.0);
    }

    #[test]
    fn mono_and_multichannel_buffers_count_frames_not_samples() {
        let (mut cb, _tx) = callback(6);
        let mut data = [0.0f32; 6 * 64];
        cb.run(&mut data, render_frame_to_f32);
        assert_eq!(cb.meter.snapshot().frames, 64);
    }
}
