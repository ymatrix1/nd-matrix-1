//! `ns4 bench --gui`: the workload and render path of the standalone GUI callback.
//!
//! The GUI renders `render_frame()` per frame (no `process_block`, one stereo pair) after
//! `FirmwareOptions::apply_exact`, with the keys pressed through MIDI. This module times that path
//! block by block from the very first block (no warm-up unless asked), reports the tail
//! (blocks over the deadline, worst block and where it was) and can replay key churn to price a
//! voice start. Timing is wall clock; the hash is deterministic and covers the left/right pair.

use crate::bench::{
    fnv1a_f32, BLOCK_FRAMES, FNV_BASIS, KEY_RANGE, SAMPLE_RATE, STANDALONE_MASTER_VOL,
};
use crate::engine::Stage4Engine;
use crate::midi::MidiEvent;
use crate::render_opts::FirmwareOptions;
use std::collections::VecDeque;
use std::time::Instant;

/// What to render.
#[derive(Debug, Clone, Copy)]
pub struct GuiBenchConfig {
    /// Layers sounding: `3` = Scene I (Organ A, Piano A, Synth A), `7` = Scene II.
    pub layers: u8,
    /// Keys held (consecutive MIDI notes from 36, velocity 100).
    pub keys: u8,
    /// Frames per callback.
    pub frames: usize,
    /// Blocks excluded from timing.
    pub warmup_blocks: usize,
    /// Timed blocks.
    pub blocks: usize,
    /// Every this many blocks release the oldest key and press a new one (`0`: static chord).
    pub churn_every: usize,
    /// Repeat the run and keep each block's fastest time (`1`: single run).
    pub repeats: usize,
    /// Firmware options, applied like the GUI's `SetFirmwareOptions`.
    pub options: FirmwareOptions,
}

impl Default for GuiBenchConfig {
    fn default() -> Self {
        Self {
            layers: 7,
            keys: 88,
            frames: BLOCK_FRAMES,
            warmup_blocks: 0,
            blocks: 3000,
            churn_every: 0,
            repeats: 1,
            options: FirmwareOptions::default(),
        }
    }
}

/// Timing of one run, milliseconds per block.
#[derive(Debug, Clone, PartialEq)]
pub struct GuiBenchReport {
    pub blocks: usize,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Block (0-based, among the timed ones) of the worst time.
    pub worst_index: usize,
    /// Blocks slower than `frames / sample_rate`.
    pub late: usize,
    pub deadline_ms: f64,
    /// First timed block (cold caches, lazy tables when `warmup_blocks == 0`).
    pub first_ms: f64,
    /// Mean of the blocks that carried a key press / of the others (churn runs).
    pub press_mean_ms: f64,
    pub quiet_mean_ms: f64,
    /// Worst press block.
    pub press_max_ms: f64,
    pub hash: u64,
    pub peak: f32,
    /// Timed frames the master bus hard-clamped at plus or minus 1.0 (audible as distortion).
    pub clamped: u64,
}

/// The engine of `cfg` before any key is pressed.
pub fn engine(cfg: &GuiBenchConfig) -> Stage4Engine {
    let mut e = Stage4Engine::new(SAMPLE_RATE);
    e.scene.scene_2_active = cfg.layers > 3;
    cfg.options.apply_exact(&mut e);
    e
}

fn note(e: &mut Stage4Engine, note: u8, on: bool) {
    e.handle_midi(if on {
        MidiEvent::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        }
    } else {
        MidiEvent::NoteOff {
            channel: 0,
            note,
            velocity: 0,
        }
    });
}

/// The held chord of a churn run: which key to release and which free key to press next.
struct Chord {
    held: VecDeque<u8>,
    cursor: u8,
}

impl Chord {
    /// `keys` consecutive notes from 36, moved down so the top stays on the 88-key keybed.
    fn new(keys: u8) -> Self {
        let (lo, hi) = (*KEY_RANGE.start(), *KEY_RANGE.end());
        let keys = keys.min(hi - lo + 1);
        let first = 36u8.min(hi + 1 - keys).max(lo);
        Self {
            held: (first..first + keys).collect(),
            cursor: first + keys - 1,
        }
    }

    /// Oldest held key released (if any) and the next key not held pressed, cycling the keybed.
    fn churn(&mut self) -> (Option<u8>, u8) {
        let released = self.held.pop_front();
        let (lo, hi) = (*KEY_RANGE.start(), *KEY_RANGE.end());
        let span = hi - lo + 1;
        let mut n = self.cursor;
        loop {
            n = lo + (n - lo + 1) % span;
            if !self.held.contains(&n) {
                break;
            }
        }
        self.cursor = n;
        self.held.push_back(n);
        (released, n)
    }
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    sorted
        .get(((sorted.len() as f64 - 1.0) * q).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// Raw per-block result of one run.
pub struct Timed {
    /// Wall time of every timed block, milliseconds.
    pub times: Vec<f64>,
    /// Whether the block carried a key press (churn runs).
    pub pressed: Vec<bool>,
    pub hash: u64,
    pub peak: f32,
    pub clamped: u64,
}

/// Renders `cfg` once on the engine `setup` leaves and times every block.
pub fn time_blocks(cfg: GuiBenchConfig, setup: impl FnOnce(&mut Stage4Engine)) -> Timed {
    let mut e = engine(&cfg);
    setup(&mut e);
    let mut chord = Chord::new(cfg.keys);
    for &k in &chord.held {
        note(&mut e, k, true);
    }
    let mut times = Vec::with_capacity(cfg.blocks);
    let mut pressed = Vec::with_capacity(cfg.blocks);
    let (mut hash, mut peak) = (FNV_BASIS, 0.0f32);
    let mut buf = vec![[0.0f32; 2]; cfg.frames];
    let mut clamp_start = 0;
    for i in 0..cfg.warmup_blocks + cfg.blocks {
        let timed = i >= cfg.warmup_blocks;
        if i == cfg.warmup_blocks {
            clamp_start = e.master_bus.clamped_frames;
        }
        let churn = cfg.keys > 0 && cfg.churn_every > 0 && i % cfg.churn_every == 0 && i > 0;
        let t0 = Instant::now();
        if churn {
            let (off, on) = chord.churn();
            if let Some(k) = off {
                note(&mut e, k, false);
            }
            note(&mut e, on, true);
        }
        for f in buf.iter_mut() {
            let [l, r] = e.render_frame();
            f[0] = l * STANDALONE_MASTER_VOL;
            f[1] = r * STANDALONE_MASTER_VOL;
        }
        if timed {
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
            pressed.push(churn);
            hash = fnv1a_f32(hash, buf.as_flattened());
            peak = buf.as_flattened().iter().fold(peak, |p, s| p.max(s.abs()));
        }
    }
    Timed {
        times,
        pressed,
        hash,
        peak,
        clamped: e.master_bus.clamped_frames - clamp_start,
    }
}

/// Renders `cfg` on the engine `setup` leaves and times every block. With `cfg.repeats > 1` the
/// run is repeated (the render is deterministic) and every block keeps its fastest time, which
/// removes scheduler and other-process noise and leaves the cost the engine itself causes.
pub fn run_with(cfg: GuiBenchConfig, setup: impl Fn(&mut Stage4Engine)) -> GuiBenchReport {
    let mut t = time_blocks(cfg, &setup);
    for _ in 1..cfg.repeats {
        let again = time_blocks(cfg, &setup);
        for (a, b) in t.times.iter_mut().zip(&again.times) {
            *a = a.min(*b);
        }
    }
    report(cfg, t)
}

/// Statistics of one [`Timed`] run.
pub fn report(cfg: GuiBenchConfig, t: Timed) -> GuiBenchReport {
    let Timed {
        times,
        pressed,
        hash,
        peak,
        clamped,
    } = t;
    let deadline_ms = cfg.frames as f64 / f64::from(SAMPLE_RATE) * 1000.0;
    let (worst_index, max_ms) =
        times
            .iter()
            .enumerate()
            .fold((0, 0.0), |a, (i, &t)| if t > a.1 { (i, t) } else { a });
    let split = |on: bool| -> Vec<f64> {
        times
            .iter()
            .zip(&pressed)
            .filter(|(_, &p)| p == on)
            .map(|(&t, _)| t)
            .collect()
    };
    let press = split(true);
    let mut sorted = times.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    GuiBenchReport {
        blocks: times.len(),
        mean_ms: mean(&times),
        p50_ms: quantile(&sorted, 0.5),
        p99_ms: quantile(&sorted, 0.99),
        max_ms,
        worst_index,
        late: times.iter().filter(|&&t| t > deadline_ms).count(),
        deadline_ms,
        first_ms: times.first().copied().unwrap_or(0.0),
        press_mean_ms: mean(&press),
        quiet_mean_ms: mean(&split(false)),
        press_max_ms: press.iter().copied().fold(0.0, f64::max),
        hash,
        peak,
        clamped,
    }
}

/// [`run_with`] on the plain engine.
pub fn run(cfg: GuiBenchConfig) -> GuiBenchReport {
    run_with(cfg, |_| {})
}

/// One report line.
pub fn format_row(label: &str, r: &GuiBenchReport) -> String {
    format!(
        "{label:<34} mean {:6.3}  p50 {:6.3}  p99 {:6.3}  max {:7.3} (block {:>4})  late {:>4}/{}  first {:6.3}",
        r.mean_ms, r.p50_ms, r.p99_ms, r.max_ms, r.worst_index, r.late, r.blocks, r.first_ms
    )
}

pub use crate::bench_gui_breakdown::{breakdown, breakdown_with};

#[cfg(test)]
mod tests {
    use super::*;

    fn quick() -> GuiBenchConfig {
        GuiBenchConfig {
            blocks: 40,
            ..GuiBenchConfig::default()
        }
    }

    #[test]
    fn run_is_deterministic_audible_and_counts_blocks() {
        let (a, b) = (run(quick()), run(quick()));
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.blocks, 40);
        assert!(a.peak > 0.0 && a.peak.is_finite());
        assert!(a.max_ms >= a.p99_ms && a.p99_ms >= a.p50_ms);
        assert!((a.deadline_ms - 2.6667).abs() < 1e-3);
        assert!(a.worst_index < a.blocks);
    }

    #[test]
    fn three_and_seven_layers_differ_and_churn_prices_key_presses() {
        let three = run(GuiBenchConfig {
            layers: 3,
            keys: 10,
            ..quick()
        });
        let seven = run(GuiBenchConfig {
            keys: 10,
            ..quick()
        });
        assert_ne!(three.hash, seven.hash, "Scene II adds layers");
        let churn = run(GuiBenchConfig {
            keys: 10,
            churn_every: 4,
            ..quick()
        });
        assert!(churn.press_mean_ms > 0.0 && churn.quiet_mean_ms > 0.0);
        assert!(churn.press_max_ms > 0.0);
        let still = run(GuiBenchConfig {
            keys: 10,
            ..quick()
        });
        assert_ne!(churn.hash, still.hash, "churn changes the sounding chord");
    }

    #[test]
    fn churn_releases_a_held_key_and_presses_a_free_one_inside_the_keybed() {
        assert!(Chord::new(0).held.is_empty());
        for keys in [1u8, 10, 87, 88] {
            let mut c = Chord::new(keys);
            assert!(c.held.iter().all(|k| KEY_RANGE.contains(k)), "{keys} keys");
            assert_eq!(c.held.len(), usize::from(keys));
            for _ in 0..200 {
                let before = c.held.clone();
                let (off, on) = c.churn();
                assert!(off.is_some());
                if let Some(k) = off {
                    assert!(before.contains(&k), "released a key that was not held");
                }
                assert!(KEY_RANGE.contains(&on));
                let others: Vec<_> = before.iter().filter(|&&k| Some(k) != off).collect();
                assert!(!others.contains(&&on), "pressed a key that is already held");
                assert_eq!(c.held.len(), usize::from(keys));
            }
        }
    }

    #[test]
    fn warmup_blocks_are_not_reported() {
        let r = run(GuiBenchConfig {
            warmup_blocks: 5,
            ..quick()
        });
        assert_eq!(r.blocks, 40);
    }
}
