//! Lock-free audio callback meter (spec `docs/specs/gui-audio.md`).
//!
//! The audio callback is the single writer; the GUI reads [`MeterSnapshot`]s and asks for a reset.
//! Everything is atomics: no allocation, lock or I/O on the audio thread
//! (`.agents/rules/dsp-and-realtime.md`). A block counts as *late* when its callback took longer
//! than the audio it produced (`frames / sample_rate`), the point where a device buffer of that
//! size would run dry unless the driver holds more buffers than one.

use super::priority::ThreadBoost;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

/// Per-block figures the audio callback hands to [`AudioMeter::record`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BlockSample {
    /// Frames in the device buffer of this callback.
    pub frames: u32,
    /// Whole callback: command drain, render, telemetry publish.
    pub total_ns: u64,
    /// Command drain share of `total_ns`.
    pub drain_ns: u64,
    /// Telemetry publish share of `total_ns`.
    pub publish_ns: u64,
    /// Peak `|sample|` written to the device.
    pub peak: f32,
    /// Samples written with `|sample| > 1.0` (above 0 dBFS, clipped by the device).
    pub over_0db: u32,
    /// Time from the start of the previous callback to the start of this one (`0` on the first).
    /// The device paces the callbacks, so a gap well above the period means the thread was not
    /// scheduled in time, whatever the render cost.
    pub gap_ns: u64,
    /// Frames the master bus hard-clamped at plus or minus 1.0 in this block (`MasterBus::clamped_frames`).
    pub clamped: u32,
}

/// Peak / over-0 dBFS accumulator for the samples of one block.
#[derive(Debug, Clone, Copy, Default)]
pub struct BlockStats {
    pub peak: f32,
    pub over_0db: u32,
}

impl BlockStats {
    /// Adds one stereo frame.
    #[inline]
    pub fn push(&mut self, l: f32, r: f32) {
        for s in [l.abs(), r.abs()] {
            // `max` ignores NaN, so a NaN sample is counted as over instead of hidden.
            self.peak = self.peak.max(s);
            self.over_0db += u32::from(s > 1.0 || s.is_nan());
        }
    }
}

/// Atomics shared by the audio callback (writer) and the GUI (reader).
#[derive(Debug, Default)]
pub struct AudioMeter {
    sample_rate: AtomicU32,
    frames: AtomicU32,
    blocks: AtomicU64,
    late: AtomicU64,
    total_ns: AtomicU64,
    /// Sum of `frames / sample_rate` over the blocks, so `load` stays right when the device
    /// changes its buffer size.
    audio_ns: AtomicU64,
    worst_ns: AtomicU64,
    worst_drain_ns: AtomicU64,
    worst_publish_ns: AtomicU64,
    worst_gap_ns: AtomicU64,
    /// Callbacks that started more than 1.5 periods after the previous one.
    slow_gaps: AtomicU64,
    peak_bits: AtomicU32,
    over_0db: AtomicU64,
    clamped: AtomicU64,
    reset_requested: AtomicBool,
    /// Packed [`ThreadBoost`] of the callback thread (written once, kept over resets).
    boost: AtomicU32,
}

/// One read of the meter.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MeterSnapshot {
    pub sample_rate: u32,
    /// Frames per callback of the latest block.
    pub frames: u32,
    pub blocks: u64,
    /// Blocks whose callback took longer than `frames / sample_rate`.
    pub late: u64,
    pub mean_ms: f64,
    /// Callback time over audio time of all blocks since the reset (`0.5` = half a core-block).
    pub load: f64,
    pub worst_ms: f64,
    pub worst_drain_ms: f64,
    pub worst_publish_ms: f64,
    /// Longest start-to-start interval of two callbacks.
    pub worst_gap_ms: f64,
    /// Callbacks that started more than 1.5 periods after the previous one (the thread was
    /// preempted or the driver stalled).
    pub slow_gaps: u64,
    pub peak: f32,
    pub over_0db: u64,
    /// Frames the master bus clamped hard at plus or minus 1.0 (audible as distortion; the
    /// over-0-dBFS count cannot see them because the clamp sits before it).
    pub clamped: u64,
    /// Scheduling class of the callback thread (empty until the first block).
    pub boost: ThreadBoost,
}

impl MeterSnapshot {
    /// Deadline of the latest block in milliseconds (`0` before the first block).
    pub fn deadline_ms(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            f64::from(self.frames) * 1000.0 / f64::from(self.sample_rate)
        }
    }

    /// Callback time as a share of the audio time it produced, over all blocks since the reset.
    pub fn load(&self) -> f64 {
        self.load
    }

    /// Peak in dBFS (`-inf` for silence).
    pub fn peak_dbfs(&self) -> f32 {
        20.0 * self.peak.log10()
    }

    /// One-line text for the status bar and the exit log.
    pub fn summary(&self) -> String {
        if self.blocks == 0 {
            return "audio: no block yet".into();
        }
        format!(
            "audio: {} frames @ {} Hz ({:.2} ms), {} blocks, late {}, load {:.0}% (mean {:.3} / worst {:.3} ms), peak {:.1} dBFS, over 0 dBFS {}, clamped {}, gap worst {:.2} ms (slow {}), {}",
            self.frames,
            self.sample_rate,
            self.deadline_ms(),
            self.blocks,
            self.late,
            self.load() * 100.0,
            self.mean_ms,
            self.worst_ms,
            self.peak_dbfs(),
            self.over_0db,
            self.clamped,
            self.worst_gap_ms,
            self.slow_gaps,
            self.boost.summary()
        )
    }
}

impl AudioMeter {
    /// Audio thread, once at stream start.
    pub fn set_sample_rate(&self, hz: u32) {
        self.sample_rate.store(hz, Relaxed);
    }

    /// Audio thread, once on the first block: what the OS reports for the callback thread.
    pub fn set_boost(&self, b: ThreadBoost) {
        self.boost.store(b.pack(), Relaxed);
    }

    /// GUI: zero the counters at the next block (the audio thread is the only one that writes).
    pub fn request_reset(&self) {
        self.reset_requested.store(true, Relaxed);
    }

    /// Audio thread, once per callback. Allocation- and lock-free.
    pub fn record(&self, b: BlockSample) {
        if self.reset_requested.swap(false, Relaxed) {
            for a in [
                &self.blocks,
                &self.late,
                &self.total_ns,
                &self.audio_ns,
                &self.worst_ns,
                &self.worst_drain_ns,
                &self.worst_publish_ns,
                &self.worst_gap_ns,
                &self.slow_gaps,
                &self.over_0db,
                &self.clamped,
            ] {
                a.store(0, Relaxed);
            }
            self.peak_bits.store(0, Relaxed);
        }
        let hz = u64::from(self.sample_rate.load(Relaxed).max(1));
        let deadline_ns = u64::from(b.frames) * 1_000_000_000 / hz;
        self.frames.store(b.frames, Relaxed);
        self.blocks.fetch_add(1, Relaxed);
        self.total_ns.fetch_add(b.total_ns, Relaxed);
        self.audio_ns.fetch_add(deadline_ns, Relaxed);
        if b.total_ns > deadline_ns {
            self.late.fetch_add(1, Relaxed);
        }
        self.worst_ns.fetch_max(b.total_ns, Relaxed);
        self.worst_drain_ns.fetch_max(b.drain_ns, Relaxed);
        self.worst_publish_ns.fetch_max(b.publish_ns, Relaxed);
        self.worst_gap_ns.fetch_max(b.gap_ns, Relaxed);
        // ASSUMED: 1.5 periods. The device paces callbacks one period apart, so 1.5 separates a
        // missed slot from jitter; unvalidated against driver behaviour, only against the probe.
        if b.gap_ns > deadline_ns + deadline_ns / 2 {
            self.slow_gaps.fetch_add(1, Relaxed);
        }
        self.over_0db.fetch_add(u64::from(b.over_0db), Relaxed);
        self.clamped.fetch_add(u64::from(b.clamped), Relaxed);
        // Non-negative floats order like their bit patterns.
        self.peak_bits.fetch_max(b.peak.max(0.0).to_bits(), Relaxed);
    }

    /// GUI: a consistent-enough copy of the counters.
    pub fn snapshot(&self) -> MeterSnapshot {
        let blocks = self.blocks.load(Relaxed);
        let (total_ns, audio_ns) = (self.total_ns.load(Relaxed), self.audio_ns.load(Relaxed));
        let ms = |ns: u64| ns as f64 / 1e6;
        MeterSnapshot {
            sample_rate: self.sample_rate.load(Relaxed),
            frames: self.frames.load(Relaxed),
            blocks,
            late: self.late.load(Relaxed),
            mean_ms: ms(total_ns) / blocks.max(1) as f64,
            load: if audio_ns > 0 {
                total_ns as f64 / audio_ns as f64
            } else {
                0.0
            },
            worst_ms: ms(self.worst_ns.load(Relaxed)),
            worst_drain_ms: ms(self.worst_drain_ns.load(Relaxed)),
            worst_publish_ms: ms(self.worst_publish_ns.load(Relaxed)),
            worst_gap_ms: ms(self.worst_gap_ns.load(Relaxed)),
            slow_gaps: self.slow_gaps.load(Relaxed),
            peak: f32::from_bits(self.peak_bits.load(Relaxed)),
            over_0db: self.over_0db.load(Relaxed),
            clamped: self.clamped.load(Relaxed),
            boost: ThreadBoost::unpack(self.boost.load(Relaxed)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(total_ns: u64, peak: f32, over: u32) -> BlockSample {
        BlockSample {
            frames: 128,
            total_ns,
            drain_ns: total_ns / 10,
            publish_ns: total_ns / 20,
            peak,
            over_0db: over,
            gap_ns: 0,
            clamped: 0,
        }
    }

    fn meter() -> AudioMeter {
        let m = AudioMeter::default();
        m.set_sample_rate(48_000);
        m
    }

    #[test]
    fn late_blocks_are_those_over_frames_divided_by_sample_rate() {
        let m = meter();
        // 128 / 48000 = 2.6667 ms.
        m.record(block(2_666_000, 0.5, 0));
        m.record(block(2_667_000, 0.5, 0));
        m.record(block(20_000_000, 0.5, 0));
        let s = m.snapshot();
        assert_eq!((s.blocks, s.late), (3, 2));
        assert!((s.deadline_ms() - 2.6667).abs() < 1e-3);
        assert!((s.worst_ms - 20.0).abs() < 1e-9);
        assert!((s.mean_ms - 8.444).abs() < 1e-2);
        assert!((s.worst_drain_ms - 2.0).abs() < 1e-9);
    }

    #[test]
    fn peak_and_over_0db_accumulate_and_reset_on_request() {
        let m = meter();
        m.record(block(1, 0.5, 0));
        m.record(block(1, 1.25, 3));
        m.record(block(1, 0.9, 2));
        let s = m.snapshot();
        assert_eq!((s.peak, s.over_0db), (1.25, 5));
        assert!((s.peak_dbfs() - 1.938).abs() < 1e-2);
        m.request_reset();
        assert_eq!(m.snapshot().blocks, 3, "reset waits for the audio thread");
        m.record(block(1, 0.1, 0));
        let s = m.snapshot();
        assert_eq!((s.blocks, s.late, s.over_0db), (1, 0, 0));
        assert_eq!(s.peak, 0.1);
    }

    #[test]
    fn block_stats_counts_over_and_nan_samples() {
        let mut st = BlockStats::default();
        st.push(0.5, -0.75);
        st.push(-1.5, 1.0);
        st.push(f32::NAN, 0.0);
        assert_eq!(st.peak, 1.5);
        assert_eq!(st.over_0db, 2);
    }

    #[test]
    fn load_is_total_callback_time_over_total_audio_time_across_buffer_sizes() {
        let m = meter();
        // 128 frames = 2.667 ms at 1.0 ms, then 1024 frames = 21.333 ms at 10.667 ms: load 0.4 / 0.5.
        m.record(block(1_000_000, 0.1, 0));
        m.record(BlockSample {
            frames: 1024,
            total_ns: 10_666_667,
            ..block(0, 0.1, 0)
        });
        let s = m.snapshot();
        assert!((s.load() - 11.666_667 / 24.0).abs() < 1e-6, "{}", s.load());
        // Old formula (mean over the latest deadline) would say 5.83 / 21.33 = 0.27.
        assert!(s.load() > 0.45);
        m.request_reset();
        m.record(block(1_333_333, 0.1, 0));
        assert!((m.snapshot().load() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn callbacks_that_start_late_are_counted_as_slow_gaps() {
        let m = meter();
        // period 2.667 ms: 2.7 ms is on time, 4.1 ms is over 1.5 periods, 30 ms is a stall.
        for gap in [0, 2_700_000, 4_100_000, 30_000_000] {
            m.record(BlockSample {
                gap_ns: gap,
                ..block(1_000_000, 0.1, 0)
            });
        }
        let s = m.snapshot();
        assert_eq!(s.slow_gaps, 2);
        assert!((s.worst_gap_ms - 30.0).abs() < 1e-9);
        assert!(s.summary().contains("gap worst 30.00 ms (slow 2)"));
        m.request_reset();
        m.record(block(1, 0.1, 0));
        assert_eq!(m.snapshot().slow_gaps, 0);
    }

    #[test]
    fn clamped_frames_accumulate_and_reset() {
        let m = meter();
        m.record(BlockSample {
            clamped: 5,
            ..block(1_000_000, 0.9, 0)
        });
        m.record(BlockSample {
            clamped: 2,
            ..block(1_000_000, 0.9, 0)
        });
        assert_eq!(m.snapshot().clamped, 7);
        assert!(m.snapshot().summary().contains("clamped 7"));
        m.request_reset();
        m.record(block(1, 0.1, 0));
        assert_eq!(m.snapshot().clamped, 0);
    }

    #[test]
    fn empty_meter_reports_nothing() {
        let s = AudioMeter::default().snapshot();
        assert_eq!(s.load(), 0.0);
        assert_eq!(s.summary(), "audio: no block yet");
    }

    #[test]
    fn summary_names_the_figures() {
        let m = meter();
        m.record(block(5_000_000, 0.5, 0));
        let t = m.snapshot().summary();
        assert!(t.contains("128 frames @ 48000 Hz"), "{t}");
        assert!(t.contains("late 1"), "{t}");
        assert!(t.contains("thread priority: n/a"), "{t}");
        m.set_boost(ThreadBoost {
            before: Some(0),
            after: Some(15),
            mmcss: true,
        });
        m.request_reset();
        m.record(block(1, 0.1, 0));
        let t = m.snapshot().summary();
        assert!(t.contains("priority 15 (was 0), MMCSS Pro Audio"), "{t}");
    }
}
