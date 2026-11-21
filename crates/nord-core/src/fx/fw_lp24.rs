//! Firmware Amp `LP24` / `HP24` (opt-in, `FirmwareFx::lp24`): the state variable pair `0x124EFA`
//! (FX core 1; core 2 copy `0x124586`) with the words the ARM sends for `kAmpMidFreq` and
//! `kAmpMid` (`docs/research/amp-types-789.md`).
//!
//! The routine runs once per call on four stereo frames, so the stage collects four frames,
//! processes them and releases them one block later: **latency 4 frames** (83 us at 48 kHz), the
//! first four output frames are silence. Allocation free and `Copy`.
//!
//! Provenance:
//! * VERIFIED: the equations (`tools/fx_model_lp24.py` against the `sharc-fx` oracle, worst output
//!   error `4.7e-07`) and the Rust port against the recorded oracle cases
//!   (`tests_fw_lp24_golden.rs`, worst deviation under `2e-5`).
//! * VERIFIED: the unit template words `L = 1.0` (`c[-25]`), `0.5` (`c[-24]`), `0.25` (`c[-23]`)
//!   and the mode words `0` (LP) / `1` (HP) (`sharc-fx` DM dump of the FX core 1 image at
//!   `0xC1A0F..0xC1A11`, `docs/research/amp-types-789.md` s2).
//! * VERIFIED: the cutoff / resonance words (`fw_lp24_tables`) at every index of the capture.
//! * VERIFIED: smoothing `K = 0.99078` (`0x3F7DA39E`): the oracle golden settles on it (a different
//!   `K` changes the `c[-26]` glide and the output words).
//! * ASSUMED: that the recurrence runs at the engine rate (the firmware's is 43,402.34 Hz, so
//!   corners are `fs / 43402.34` higher at other rates, as for the other ports) and that the
//!   Mid knob maps to `kAmpMid` counts as `res / 10 * 120 - 60` (`LayerFxChain::process_fw_lp24`).

use super::fw_lp24_tables as tab;

/// VERIFIED: coefficient smoothing factor (`0x3F7DA39E`, oracle fit of the `c[-26]` glide).
const K: f32 = f32::from_bits(0x3F7D_A39E);
/// VERIFIED: clip bound `L = c[-25]` of the unit template.
const LIMIT: f32 = 1.0;
/// VERIFIED: template constants `c[-24]`, `c[-23]` (HP gain term).
const C24: f32 = 0.5;
const C23: f32 = 0.25;

/// Filter mode word `c[-4]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lp24Mode {
    /// `LP24` (`kAmpTypeNew` 8, mode word 0).
    Low,
    /// `HP24` (`kAmpTypeNew` 9, mode word 1).
    High,
}

/// Targets and smoothed copies of the context words.
#[derive(Debug, Clone, Copy)]
struct Words {
    /// `c[-32]` / `c[-31]`: cutoff target and smoothed state.
    cut: [f32; 2],
    /// `c[-29]` / `c[-27]`: resonance target and smoothed state.
    res: [f32; 2],
    /// `c[-28]` / `c[-26]`: stage gain target and smoothed state.
    gain: [f32; 2],
}

/// Firmware LP24 / HP24 stage: the pair plus the 4-frame buffers.
#[derive(Debug, Clone, Copy)]
pub(super) struct FwLp24 {
    mode: Lp24Mode,
    words: Words,
    /// Six states per channel: `c[-16, -14, -12, -10, -8, -6]`.
    st: [[f32; 6]; 2],
    input: [[f32; 2]; 4],
    output: [[f32; 2]; 4],
    pos: usize,
}

impl FwLp24 {
    /// Stage for cutoff index `freq` (`0..=127`) and Mid knob counts `mid` (`-60..=60`), settled.
    pub(super) fn new(mode: Lp24Mode, freq: f32, mid: f32) -> Self {
        let mut s = Self {
            mode,
            words: Words {
                cut: [0.0; 2],
                res: [0.0; 2],
                gain: [0.0; 2],
            },
            st: [[0.0; 6]; 2],
            input: [[0.0; 2]; 4],
            output: [[0.0; 2]; 4],
            pos: 0,
        };
        s.set(freq, mid, true);
        s
    }

    /// The mode this stage was built for.
    pub(super) fn mode(&self) -> Lp24Mode {
        self.mode
    }

    /// Moves the targets; the routine's own smoother glides there unless `settle`.
    pub(super) fn set(&mut self, freq: f32, mid: f32, settle: bool) {
        let (cut, res, gain) = (
            tab::cutoff_word(freq),
            tab::res_word(mid),
            tab::gain_word(mid),
        );
        self.words.cut[0] = cut;
        self.words.res[0] = res;
        self.words.gain[0] = gain;
        if settle {
            self.words.cut[1] = cut;
            self.words.res[1] = res;
            self.words.gain[1] = gain;
        }
    }

    /// Pushes one frame and returns the frame of the previous block at the same position.
    #[inline]
    pub(super) fn process_frame(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let out = self.output[self.pos];
        self.input[self.pos] = frame;
        self.pos += 1;
        if self.pos == 4 {
            self.pos = 0;
            let block = self.input;
            self.output = self.run_block(block);
        }
        out
    }

    /// One routine call: smooths the words once, then runs the four frames.
    fn run_block(&mut self, frames: [[f32; 2]; 4]) -> [[f32; 2]; 4] {
        let w = &mut self.words;
        w.cut[1] = w.cut[0] + K * (w.cut[1] - w.cut[0]);
        w.res[1] = w.res[0] + K * (w.res[1] - w.res[0]);
        w.gain[1] = w.gain[0] + K * (w.gain[1] - w.gain[0]);
        let x = (16.0 * w.cut[1]).clamp(-4.0, 4.0);
        let g2 = 2.0 * w.gain[1];
        let inv = 1.0 / (LIMIT + 4.0 * x * w.res[1] + x * x).sqrt();
        let a1 = 2.0 * x * inv;
        let b = 4.0 * w.res[1] * inv;
        let h = LIMIT - (b * a1 * C24 + a1 * a1 * C23);
        let mut out = [[0.0f32; 2]; 4];
        for ch in 0..2 {
            let s = &mut self.st[ch];
            for (i, f) in frames.iter().enumerate() {
                out[i][ch] = match self.mode {
                    Lp24Mode::Low => {
                        let y1 = lp_stage(f[ch], g2, a1, b, &mut s[0..3]);
                        lp_stage(y1, 1.0, a1, b, &mut s[3..6])
                    }
                    Lp24Mode::High => {
                        let e1 = hp_stage(f[ch], h, a1, b, &mut s[0..2]).1;
                        hp_stage(e1, h, a1, b, &mut s[3..5]).0
                    }
                };
            }
        }
        out
    }

    /// Test access: replaces the smoothed words and states with the oracle context.
    #[cfg(test)]
    pub(super) fn load(&mut self, words: [[f32; 2]; 3], st: [[f32; 6]; 2]) {
        self.words = Words {
            cut: words[0],
            res: words[1],
            gain: words[2],
        };
        self.st = st;
    }

    /// Test access: runs one block directly (no buffering).
    #[cfg(test)]
    pub(super) fn block(&mut self, frames: [[f32; 2]; 4]) -> [[f32; 2]; 4] {
        self.run_block(frames)
    }
}

#[inline]
fn clip(v: f32) -> f32 {
    v.clamp(-LIMIT, LIMIT)
}

/// LP stage `(x g)` on states `[s1, s2, s3]`; returns the output.
#[inline]
fn lp_stage(x: f32, g: f32, a1: f32, b: f32, s: &mut [f32]) -> f32 {
    let f = x * g;
    let (s1, s2, s3) = (s[0], s[1], s[2]);
    let t3 = s3 + s1 * a1;
    let e = (f + s2) * 0.5 - t3 - s1 * b;
    s[2] = t3;
    s[1] = f;
    s[0] = clip(s1 + e * a1);
    (t3 + s3) * 0.5
}

/// HP stage on states `[s1, s2]`; returns `(F9, e)`.
#[inline]
fn hp_stage(x: f32, h: f32, a1: f32, b: f32, s: &mut [f32]) -> (f32, f32) {
    let (s1, s2) = (s[0], s[1]);
    let f13 = s2 + s1 * a1;
    let f9 = x * h - f13;
    let e = f9 - s1 * b;
    s[1] = f13;
    s[0] = clip(s1 + a1 * e);
    (f9, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mode: Lp24Mode, freq: f32, mid: f32, cycles: f32) -> f32 {
        let mut st = FwLp24::new(mode, freq, mid);
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for k in 0..30_000 {
            let x = 0.1 * (2.0 * std::f32::consts::PI * cycles * k as f32).sin();
            let y = st.process_frame([x, x])[0];
            if k > 20_000 {
                num += (y as f64).powi(2);
                den += (x as f64).powi(2);
            }
        }
        (10.0 * (num / den).log10()) as f32
    }

    #[test]
    fn low_pass_passes_low_and_rejects_high() {
        let low = run(Lp24Mode::Low, 64.0, 0.0, 0.002);
        let high = run(Lp24Mode::Low, 64.0, 0.0, 0.3);
        assert!(low > -3.0, "passband {low}");
        assert!(high < low - 30.0, "stopband {high}");
    }

    #[test]
    fn high_pass_rejects_low_and_passes_high() {
        let low = run(Lp24Mode::High, 64.0, 0.0, 0.0005);
        let high = run(Lp24Mode::High, 64.0, 0.0, 0.3);
        assert!(high > -3.0, "passband {high}");
        assert!(low < high - 30.0, "stopband {low}");
    }

    #[test]
    fn first_block_is_silent_then_the_latency_is_four_frames() {
        let mut st = FwLp24::new(Lp24Mode::Low, 100.0, 0.0);
        let first: Vec<f32> = (0..4).map(|_| st.process_frame([1.0, 1.0])[0]).collect();
        assert_eq!(first, [0.0; 4]);
        let next: Vec<f32> = (0..4).map(|_| st.process_frame([1.0, 1.0])[0]).collect();
        assert!(next.iter().any(|v| *v != 0.0), "{next:?}");
    }

    #[test]
    fn states_stay_bounded_and_finite_at_full_resonance() {
        for mode in [Lp24Mode::Low, Lp24Mode::High] {
            let mut st = FwLp24::new(mode, 127.0, -60.0);
            for k in 0..50_000 {
                let x = if k % 97 < 3 { 1.0 } else { -0.3 };
                let y = st.process_frame([x, -x]);
                assert!(y[0].is_finite() && y[1].is_finite());
            }
            assert!(st.st.iter().flatten().all(|v| v.abs() <= LIMIT + 1e-6));
        }
    }

    #[test]
    fn set_without_settle_glides_with_the_smoother() {
        let mut st = FwLp24::new(Lp24Mode::Low, 20.0, 0.0);
        st.set(100.0, 0.0, false);
        let before = st.words.cut[1];
        let _ = st.run_block([[0.0; 2]; 4]);
        assert!(st.words.cut[1] > before && st.words.cut[1] < st.words.cut[0]);
    }
}
