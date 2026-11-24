//! B3 Bass stage: the firmware generator `0x121BBE` and processor `0x122296` at the DSP rate (opt-in).
//!
//! Spec: `docs/specs/organ-engine.md` section 3.1.9. Research: `docs/research/organ-farfisa-bass.md`
//! s15. Opt-in (`OrganLayer::set_b3_bass_processor`, `--b3-bass-processor on`), only on the `B3`
//! model in Bass mode and without a processor tail (the stage is the tail of this model).
//!
//! The stage runs the decoded firmware routines (`b3_bass_fw`: scanner, lattice sections, delay
//! taps, biquads, DC blocker, 40 bit float) on a DM image of the installed model 5, both built at
//! setup time from the user's own OS update file (D128, no file: no stage), and feeds them
//! the key-down state the ARM would write: the weight windows and `G`
//! ([`super::b3_bass_state`]), the generator weights ([`super::b3_bass_gen`]) and the voice records
//! ([`super::b3_bass_voice`]). Two output samples per call at the DSP frame rate
//! ([`super::farfisa_wheels::DSP_FRAME_RATE`]) are resampled to the host rate (cubic Hermite).
//!
//! Tags: routines and window / weight / `G` state VERIFIED (bit exact against `sharc-chain` for the
//! recorded states); voice record jitter, the slot reuse, `DM 0xB44CB`, the drawbar and percussion
//! settings (the tables are the `888888888` key-down) and the `G` resync after a key-up ASSUMED.

use super::b3_bass_fw::{from_env, BassImage, Machine};
use super::b3_bass_gen::GenWeights;
use super::b3_bass_state::BassState;
use super::b3_bass_voice::{word_addr, VoiceTable, SLOTS};
use super::farfisa_wheels::DSP_FRAME_RATE;
use super::{OrganLayer, OrganModel, ToneWheelSource};

/// Keys `24..=108` carry key-down tables.
const MIN_KEY: i32 = 24;
const MAX_KEY: i32 = 108;
/// Window 1 / window 2 / generator weight / `G` words (VERIFIED, research s11 / s12).
const W1_ADDR: u32 = 0x9_51F1;
const W2_ADDR: u32 = 0x9_5241;
const GEN_ADDR: u32 = 0x9_5122;
const G_ADDR: u32 = 0xB_44C9;
/// Generator weight wheels `36..=114`.
const GEN_NOTES: std::ops::RangeInclusive<usize> = 36..=114;
/// ASSUMED: calls to keep running after the last key-up (about 0.46 s) before the stage idles.
const RING_DOWN_CALLS: u32 = 10_000;

/// Catmull-Rom interpolation between `h[1]` and `h[2]` at `mu` in `[0, 1)`.
pub(super) fn cubic(h: &[f32; 4], mu: f32) -> f32 {
    let (a, b, c, d) = (h[0], h[1], h[2], h[3]);
    let m1 = 0.5 * (c - a);
    let m2 = 0.5 * (d - b);
    let (mu2, mu3) = (mu * mu, mu * mu * mu);
    (2.0 * mu3 - 3.0 * mu2 + 1.0) * b
        + (mu3 - 2.0 * mu2 + mu) * m1
        + (-2.0 * mu3 + 3.0 * mu2) * c
        + (mu3 - mu2) * m2
}

/// State of the stage: the firmware machine, the key tables and the resampler.
#[derive(Debug, Clone)]
pub(super) struct B3BassStage {
    machine: Machine,
    state: BassState,
    gen: GenWeights,
    voices: VoiceTable,
    /// Transposed key registered by `note_on` of each note (`-1` = none).
    keys: [i16; 128],
    /// Key-downs since the last idle period (selects the voice slot).
    presses: usize,
    /// Calls left to run after the last key-up.
    cooldown: u32,
    /// Output samples of the last call not yet consumed (`pending` of them, oldest first).
    queue: [f32; 2],
    pending: usize,
    hist: [f32; 4],
    /// Position of the next host sample between `hist[1]` and `hist[2]`.
    pos: f64,
}

impl B3BassStage {
    /// Builds the machine from the image of the user's OS update file (setup time only:
    /// allocates and compiles the rows).
    pub(super) fn new(image: &BassImage) -> Self {
        let mut machine = Machine::new();
        machine.load_image(&image.csv);
        machine.set_programs(Some(&image.gen), &image.proc);
        Self {
            machine,
            state: BassState::new(),
            gen: GenWeights::new(),
            voices: VoiceTable::new(),
            keys: [-1; 128],
            presses: 0,
            cooldown: 0,
            queue: [0.0; 2],
            pending: 0,
            hist: [0.0; 4],
            pos: 0.0,
        }
    }

    /// Writes the held-key state words into the machine DM.
    fn push_state(&mut self) {
        let m = &mut self.machine;
        let (w1, w2) = self.state.windows();
        for (k, &v) in w1.iter().enumerate() {
            m.wr_f(W1_ADDR + k as u32, v);
        }
        for (k, &v) in w2.iter().enumerate() {
            m.wr_f(W2_ADDR + k as u32, v);
        }
        for note in GEN_NOTES {
            m.wr_f(GEN_ADDR + (note - 36) as u32, self.gen.weight(note));
        }
        m.wr_f(G_ADDR, self.state.g());
    }

    /// Key-down of `note` at the transposed `key`.
    pub(super) fn note_on(&mut self, note: usize, key: i32) {
        if !(MIN_KEY..=MAX_KEY).contains(&key) || self.keys[note] >= 0 {
            return;
        }
        if !self.keys.iter().any(|&k| k >= 0) {
            self.presses = 0;
        }
        self.keys[note] = key as i16;
        self.state.key_down(key as u8);
        self.gen.key_down(key as usize);
        if let Some(words) = self.voices.words(key as usize) {
            let slot = self.presses % SLOTS;
            for (i, &w) in words.iter().enumerate() {
                self.machine.wr(word_addr(slot, i), w);
            }
        }
        self.presses += 1;
        self.push_state();
        self.cooldown = u32::MAX;
    }

    /// Key-up of `note` (the key registered at its key-down).
    pub(super) fn note_off(&mut self, note: usize) {
        let key = self.keys[note];
        if key < 0 {
            return;
        }
        self.keys[note] = -1;
        self.state.key_up(key as u8);
        self.gen.key_up(key as usize);
        self.push_state();
        if !self.keys.iter().any(|&k| k >= 0) {
            self.cooldown = RING_DOWN_CALLS;
        }
    }

    /// `true` while a key is held or the machine still rings down.
    pub(super) fn active(&self) -> bool {
        self.cooldown > 0
    }

    /// One call of the firmware (two output samples at the DSP rate).
    fn call(&mut self) -> [f32; 2] {
        if self.cooldown != u32::MAX {
            self.cooldown = self.cooldown.saturating_sub(1);
        }
        self.machine.step()
    }

    /// Next output sample at the DSP rate.
    fn dsp_sample(&mut self) -> f32 {
        if self.pending == 0 {
            self.queue = self.call();
            self.pending = 2;
        }
        let v = self.queue[2 - self.pending];
        self.pending -= 1;
        v
    }

    /// One output sample (before the level law) at the host rate.
    pub(super) fn process(&mut self, sample_rate: f32) -> f32 {
        if self.cooldown == 0 && self.pending == 0 {
            return 0.0;
        }
        while self.pos >= 1.0 {
            self.hist = [self.hist[1], self.hist[2], self.hist[3], self.dsp_sample()];
            self.pos -= 1.0;
        }
        let y = cubic(&self.hist, self.pos as f32);
        self.pos += DSP_FRAME_RATE / f64::from(sample_rate);
        y
    }

    /// `n` raw output samples at the DSP rate (tests).
    #[cfg(test)]
    pub(super) fn dsp_block(&mut self, n: usize) -> Vec<f32> {
        (0..n).map(|_| self.dsp_sample()).collect()
    }
}

impl OrganLayer {
    /// Switches the Bass processor stage (setup time: allocates the stage on first use). Turning
    /// it on selects the plain wheel tables (no B3 tail): the stage replaces the tail for this
    /// model.
    ///
    /// The routines and their DM image come from the user's own OS update file, named by
    /// `NS4_WAVE_TABLES` or the saved app setting (D128, D129). Without a readable file the stage
    /// is not built: `b3_bass_processor` stays as requested, [`Self::b3_bass_firmware_loaded`] is
    /// `false`, the tail is left alone and the layer plays the stage-off model (D22).
    pub fn set_b3_bass_processor(&mut self, on: bool) {
        let image = if on && self.b3_bass.is_none() {
            from_env().ok().flatten()
        } else {
            None
        };
        self.set_b3_bass_processor_with(on, image.as_deref());
    }

    /// [`Self::set_b3_bass_processor`] with an explicit image (`None`: no OS update file).
    pub(super) fn set_b3_bass_processor_with(&mut self, on: bool, image: Option<&BassImage>) {
        self.b3_bass_processor = on;
        if on {
            if let (None, Some(image)) = (&self.b3_bass, image) {
                self.b3_bass = Some(Box::new(B3BassStage::new(image)));
            }
            if self.b3_bass.is_some() {
                self.set_b3_tail(false);
            }
        }
    }

    /// `true` when the firmware Bass stage exists (the OS update file was read); `false` when it
    /// was switched on without one and the stage-off model plays.
    pub fn b3_bass_firmware_loaded(&self) -> bool {
        self.b3_bass.is_some()
    }

    /// Whether the Bass processor stage is switched on (`--b3-bass-processor on`).
    pub fn b3_bass_processor_on(&self) -> bool {
        self.b3_bass_processor
    }

    /// `true` when the Bass stage replaces the drawbar sum of this layer.
    pub(super) fn b3_bass_active(&self) -> bool {
        self.b3_bass_processor
            && self.b3_bass.is_some()
            && self.model == OrganModel::B3
            && self.b3_bass_mode
            && !self.b3_tail
            && self.effective_tone_source() == ToneWheelSource::B3Wheels
    }

    /// `true` while the stage still carries signal.
    pub(super) fn b3_bass_sounding(&self) -> bool {
        self.b3_bass_active() && self.b3_bass.as_ref().is_some_and(|b| b.active())
    }

    /// Key event hook of `note_on` / `note_off`.
    pub(super) fn b3_bass_key(&mut self, note: usize, down: bool) {
        let oct = i32::from(self.octave_shift.clamp(-2, 2)) * 12;
        if let Some(b) = self.b3_bass.as_mut() {
            if down {
                b.note_on(note, note as i32 + oct);
            } else {
                b.note_off(note);
            }
        }
    }

    /// Output of the Bass stage (level law of the B3 weight path).
    pub(super) fn sum_b3_bass(&mut self) -> f32 {
        let level = super::b3_weights::DRY_GAIN * self.level_law().bus_gain();
        let fs = self.sample_rate;
        level * self.b3_bass.as_mut().map_or(0.0, |b| b.process(fs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cubic_reproduces_a_line_and_the_end_points() {
        let h = [0.0, 1.0, 2.0, 3.0];
        assert!((cubic(&h, 0.5) - 1.5).abs() < 1e-6);
        assert_eq!(cubic(&h, 0.0), 1.0);
        assert!((cubic(&h, 0.999_999) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn voice_slots_cycle_over_five_presses() {
        let mut s = B3BassStage::new(&super::super::b3_bass_fw::synthetic_image());
        for (n, k) in [
            (60usize, 60i32),
            (64, 64),
            (67, 67),
            (72, 72),
            (76, 76),
            (79, 79),
        ] {
            s.note_on(n, k);
        }
        // the sixth press reuses slot 0: the record of key 79 sits at `0x954E6`
        let first = s.machine.rd(super::super::b3_bass_voice::word_addr(0, 4));
        let want = s.voices.words(79).unwrap()[4];
        assert_eq!(first, want);
        assert!(s.active());
    }
}
