//! The body of the CPAL output callback, free of `cpal` types so it can be tested and metered.
//!
//! Per callback: drain the command queue, render every frame, publish telemetry, then record the
//! timings and the output level in the [`AudioMeter`]. No allocation, lock or I/O here
//! (`.agents/rules/dsp-and-realtime.md`); `Instant::now` is a clock read, not a syscall that blocks.

use super::garbage::{Bin, Garbage};
use super::meter::{AudioMeter, BlockSample, BlockStats};
use super::priority::boost_current_thread;
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
    /// The first block raises the thread priority once (`priority.rs`).
    boosted: bool,
    /// `--no-thread-boost`: leave the thread as the driver made it (comparison runs).
    boost_enabled: bool,
    last_start: Option<Instant>,
    /// Where spent program boxes and replaced sound banks go instead of being freed here.
    bin: Bin,
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
            boosted: false,
            boost_enabled: true,
            last_start: None,
            bin: Bin::inline(),
        }
    }

    /// Routes values the callback would free to `bin` (a reaper thread), see `garbage.rs`.
    pub fn with_garbage(mut self, bin: Bin) -> Self {
        self.bin = bin;
        self
    }

    /// Switches the one-time thread boost off (`--no-thread-boost`).
    pub fn with_thread_boost(mut self, on: bool) -> Self {
        self.boost_enabled = on;
        self
    }

    /// Fills `data` (interleaved, `channels` wide) and records the block in the meter.
    /// `write` stores one stereo frame in the device sample format.
    pub fn run<S>(&mut self, data: &mut [S], write: fn(usize, f32, f32, &mut [S])) {
        let start = Instant::now();
        let gap_ns = self
            .last_start
            .map_or(0, |p| start.duration_since(p).as_nanos() as u64);
        self.last_start = Some(start);
        if !self.boosted {
            self.boosted = true;
            if self.boost_enabled {
                self.meter.set_boost(boost_current_thread());
            }
        }
        // The one-time boost is not part of the first block's timing.
        let t0 = Instant::now();
        while let Ok(cmd) = self.rx.try_recv() {
            apply_realtime(cmd, &mut self.engine, &mut self.master_vol, &self.bin);
        }
        let t1 = Instant::now();
        let mut stats = BlockStats::default();
        let clamped_before = self.engine.master_bus.clamped_frames;
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
            gap_ns,
            clamped: (self.engine.master_bus.clamped_frames - clamped_before) as u32,
        });
    }
}

/// Applies one command inside the audio callback. A control operation (allocating) that reached
/// this thread despite the control link is dropped and counted, never executed here.
pub fn apply_realtime(
    cmd: AudioCommand,
    engine: &mut Stage4Engine,
    master_vol: &mut f32,
    bin: &Bin,
) {
    match cmd {
        AudioCommand::Program(e) if e.is_control_op() => {
            engine.note_retire_overflow();
            std::mem::forget(e);
        }
        // The spent box / replaced bank is freed by the reaper, not here.
        AudioCommand::LoadProgram(prog) => {
            engine.apply_program(&prog);
            bin.put(Garbage::Program(prog));
        }
        AudioCommand::LoadPianoSound { slot_b, sound } => {
            let layer = if slot_b {
                &mut engine.piano_b
            } else {
                &mut engine.piano_a
            };
            bin.put(Garbage::Piano(layer.replace_sound(sound)));
        }
        AudioCommand::LoadSynthSample { layer_idx, sound } => {
            let layer = match layer_idx {
                0 => &mut engine.synth_a,
                1 => &mut engine.synth_b,
                2 => &mut engine.synth_c,
                _ => return,
            };
            bin.put(Garbage::Synth(layer.replace_sample(sound)));
        }
        other => apply_audio_command(other, engine, master_vol),
    }
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

    #[test]
    fn the_first_block_boosts_the_thread_once_and_the_opt_out_leaves_it_alone() {
        let (cb, _tx) = callback(2);
        let mut cb = cb.with_thread_boost(false);
        let mut data = [0.0f32; 256];
        cb.run(&mut data, render_frame_to_f32);
        assert_eq!(cb.meter.snapshot().boost, Default::default(), "opted out");
        // The boosted run happens on a scratch thread so the test thread keeps its priority.
        let boosted = std::thread::spawn(|| {
            let (mut cb, _tx) = callback(2);
            let mut data = [0.0f32; 256];
            cb.run(&mut data, render_frame_to_f32);
            cb.run(&mut data, render_frame_to_f32);
            cb.meter.snapshot().boost
        })
        .join()
        .unwrap();
        #[cfg(windows)]
        assert!(boosted.is_raised() && boosted.mmcss, "{boosted:?}");
        #[cfg(not(windows))]
        assert_eq!(boosted, Default::default());
    }

    #[test]
    fn the_gap_between_callbacks_is_metered_from_the_second_block() {
        let (mut cb, _tx) = callback(2);
        let mut data = [0.0f32; 256];
        cb.run(&mut data, render_frame_to_f32);
        assert_eq!(cb.meter.snapshot().worst_gap_ms, 0.0, "no gap on the first");
        std::thread::sleep(std::time::Duration::from_millis(8));
        cb.run(&mut data, render_frame_to_f32);
        let s = cb.meter.snapshot();
        assert!(s.worst_gap_ms >= 8.0, "{}", s.worst_gap_ms);
        assert_eq!(
            s.slow_gaps, 1,
            "128 frames = 2.7 ms period, 8 ms gap is slow"
        );
    }

    #[test]
    fn spent_program_boxes_and_sound_banks_go_to_the_bin_not_the_free_list() {
        let (bin, reaper) = super::super::garbage::channel();
        let mut engine = standalone_engine(48_000.0);
        let mut vol = 0.85;
        let load = AudioCommand::LoadProgram(Box::default());
        apply_realtime(load, &mut engine, &mut vol, &bin);
        let piano = AudioCommand::LoadPianoSound {
            slot_b: true,
            sound: None,
        };
        apply_realtime(piano, &mut engine, &mut vol, &bin);
        let synth = AudioCommand::LoadSynthSample {
            layer_idx: 2,
            sound: None,
        };
        apply_realtime(synth, &mut engine, &mut vol, &bin);
        let got: Vec<_> = reaper.try_iter().collect();
        assert_eq!(got.len(), 3);
        assert!(matches!(got[0], Garbage::Program(_)));
        assert!(matches!(got[1], Garbage::Piano(_)));
        assert!(matches!(got[2], Garbage::Synth(_)));
        // A bad layer index is ignored and queues nothing.
        let bad = AudioCommand::LoadSynthSample {
            layer_idx: 9,
            sound: None,
        };
        apply_realtime(bad, &mut engine, &mut vol, &bin);
        assert_eq!(reaper.try_iter().count(), 0);
    }
}
