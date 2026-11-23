//! Farfisa key stage: the firmware processor `0x122B85` at the DSP rate (opt-in).
//!
//! Spec: `docs/specs/organ-engine.md` section 3.1.8. Research: `docs/research/organ-farfisa-bass.md`
//! s16. Opt-in (`OrganLayer::set_farfisa_processor`, `--farfisa-processor on`), only on the
//! `Farfisa` model with `ToneWheelSource::FarfisaWheels`.
//!
//! The stage runs the decoded routine ([`super::farfisa_fw`]: weighted sums over the ring, the
//! two lattice tables, the output gains, the DC blocker, 40 bit float) on a DM image of the
//! installed model 2. Its input ring is the divider of [`super::farfisa_wheels`] run at the DSP
//! frame rate (two frames per call: frame 0 at `DM 0xC2B52`, frame 1 at `0xC2BB4`); the key-down
//! writes what the ARM writes: the weight `1/48` at the three sums and the shared bias words.
//! Two output samples per call are resampled to the host rate (cubic Hermite, as the Bass stage).
//!
//! Tags: the routine and the weights VERIFIED (bit exact against `sharc-chain` with the oracle's
//! own ring, both lattice tables); the ring from `FarfisaWheels` is within its own golden bound
//! (2e-4, `farfisa_wheels.rs`), not bit exact; the lattice flag 0 (default), the bias words held
//! while any key is down, the hard key-up and the 0.46 s ring-down are ASSUMED. Not modelled: the
//! Farfisa tab selection and the vibrato (no captured registers), an idle divider is frozen.

use super::b3_bass_stage::cubic;
use super::farfisa_fw::{from_env, FarfisaFw, FarfisaImage, RING_LEN};
use super::farfisa_wheels::{rows_of, FarfisaWheels, DSP_FRAME_RATE, OSCILLATORS, ROWS};
use super::{OrganLayer, OrganModel, ToneWheelSource};

/// Keys `24..=108` carry key-down words inside their sums (VERIFIED, research s7).
const MIN_KEY: i32 = 24;
const MAX_KEY: i32 = 108;
/// ASSUMED: calls to keep running after the last key-up (about 0.46 s) before the stage idles.
const RING_DOWN_CALLS: u32 = 10_000;

/// State of the key stage: the firmware machine, the divider and the resampler.
#[derive(Debug, Clone)]
pub(super) struct FarfisaKeys {
    fw: FarfisaFw,
    wheels: FarfisaWheels,
    /// Fine tune the divider is tuned to (cents).
    tuned: f32,
    /// Transposed key registered by `note_on` of each note (`-1` = none).
    keys: [i16; 128],
    held: u32,
    /// Calls left to run after the last key-up (`u32::MAX` while a key is held).
    cooldown: u32,
    rows: [[f32; OSCILLATORS]; ROWS],
    ring: [f32; RING_LEN],
    queue: [f32; 2],
    pending: usize,
    hist: [f32; 4],
    pos: f64,
}

impl FarfisaKeys {
    /// All keys off on the image built from the user's OS update file (allocates the DM image;
    /// setup time only).
    pub(super) fn new(image: &FarfisaImage) -> Self {
        Self {
            fw: FarfisaFw::new(image),
            wheels: FarfisaWheels::new(),
            tuned: 0.0,
            keys: [-1; 128],
            held: 0,
            cooldown: 0,
            rows: [[0.0; OSCILLATORS]; ROWS],
            ring: [0.0; RING_LEN],
            queue: [0.0; 2],
            pending: 0,
            hist: [0.0; 4],
            pos: 0.0,
        }
    }

    /// Key-down of `note` at the transposed `key` with the velocity gain `gain` (`0..=1`).
    pub(super) fn note_on(&mut self, note: usize, key: i32, gain: f32) {
        if !(MIN_KEY..=MAX_KEY).contains(&key) || self.keys[note] >= 0 {
            return;
        }
        self.keys[note] = key as i16;
        self.fw.set_key(key as u32, gain);
        self.held += 1;
        self.fw.set_bias(true);
        self.cooldown = u32::MAX;
    }

    /// Key-up of `note` (the key registered at its key-down).
    pub(super) fn note_off(&mut self, note: usize) {
        let key = self.keys[note];
        if key < 0 {
            return;
        }
        self.keys[note] = -1;
        self.fw.set_key(key as u32, 0.0);
        self.held -= 1;
        if self.held == 0 {
            self.fw.set_bias(false);
            self.cooldown = RING_DOWN_CALLS;
        }
    }

    /// `true` while a key is held or the machine still rings down.
    pub(super) fn active(&self) -> bool {
        self.cooldown > 0 || self.pending > 0
    }

    /// Retunes the divider (`fine` cents; changes are rare, setup-time cost only).
    fn tune(&mut self, fine: f32) {
        if (fine - self.tuned).abs() > 1e-6 {
            self.wheels.retune(DSP_FRAME_RATE as f32, fine);
            self.tuned = fine;
        }
    }

    /// One call of the firmware: two divider frames in, two output samples out.
    fn call(&mut self) -> [f32; 2] {
        if self.cooldown != u32::MAX {
            self.cooldown = self.cooldown.saturating_sub(1);
        }
        for frame in 0..2 {
            self.wheels.advance_rows(&mut self.rows);
            for (r, row) in self.rows.iter().enumerate() {
                for (n, &y) in row.iter().enumerate().take(OSCILLATORS) {
                    if r < rows_of(n) {
                        self.ring[12 * r + n] = y;
                    }
                }
            }
            self.fw.set_ring(frame, &self.ring);
        }
        self.fw.step()
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
    pub(super) fn process(&mut self, sample_rate: f32, fine: f32) -> f32 {
        if !self.active() {
            return 0.0;
        }
        self.tune(fine);
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
    /// Switches the Farfisa processor stage (setup time: allocates the stage on first use).
    ///
    /// The routine and its DM image come from the user's own OS update file, named by
    /// `NS4_WAVE_TABLES` or the saved app setting (D128, D129). Without a readable file the stage
    /// is not built: `farfisa_processor` stays as requested, [`Self::farfisa_firmware_loaded`] is
    /// `false` and the layer plays its non-firmware Farfisa model (D22).
    pub fn set_farfisa_processor(&mut self, on: bool) {
        let image = if on && self.farfisa_keys.is_none() {
            from_env().ok().flatten()
        } else {
            None
        };
        self.set_farfisa_processor_with(on, image.as_deref());
    }

    /// [`Self::set_farfisa_processor`] with an explicit image (`None`: no OS update file).
    pub(super) fn set_farfisa_processor_with(&mut self, on: bool, image: Option<&FarfisaImage>) {
        self.farfisa_processor = on;
        if let (true, None, Some(image)) = (on, &self.farfisa_keys, image) {
            self.farfisa_keys = Some(Box::new(FarfisaKeys::new(image)));
        }
    }

    /// `true` when the firmware Farfisa stage exists (the OS update file was read); `false` when
    /// it was switched on without one and the non-firmware model plays.
    pub fn farfisa_firmware_loaded(&self) -> bool {
        self.farfisa_keys.is_some()
    }

    /// `true` when the Farfisa processor stage replaces the drawbar sum of this layer.
    pub(super) fn farfisa_keys_active(&self) -> bool {
        self.farfisa_processor
            && self.farfisa_keys.is_some()
            && self.model == OrganModel::Farfisa
            && self.effective_tone_source() == ToneWheelSource::FarfisaWheels
    }

    /// Key event hook of `note_on_with_gain` / `note_off`.
    pub(super) fn farfisa_key(&mut self, note: usize, down: bool, gain: f32) {
        let oct = i32::from(self.octave_shift.clamp(-2, 2)) * 12;
        if let Some(k) = self.farfisa_keys.as_mut() {
            if down {
                k.note_on(note, note as i32 + oct, gain);
            } else {
                k.note_off(note);
            }
        }
    }

    /// `true` while the stage still carries signal.
    pub(super) fn farfisa_keys_sounding(&self) -> bool {
        self.farfisa_keys_active() && self.farfisa_keys.as_ref().is_some_and(|k| k.active())
    }

    /// Output of the key stage (level law of the firmware paths, `L`).
    pub(super) fn sum_farfisa_keys(&mut self) -> f32 {
        let level = self.weight_level * self.level_law().bus_gain();
        let (fs, fine) = (
            self.sample_rate,
            super::vibrato_bank::clamp_fine_tune(self.effective_fine_cents()),
        );
        level
            * self
                .farfisa_keys
                .as_mut()
                .map_or(0.0, |k| k.process(fs, fine))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_stage_is_silent_and_a_key_makes_it_active_until_the_ring_down_ends() {
        let mut s = FarfisaKeys::new(&crate::organ::farfisa_fw::synthetic_image());
        assert!(!s.active());
        assert_eq!(s.process(48_000.0, 0.0), 0.0);
        s.note_on(60, 60, 1.0);
        assert!(s.active());
        s.note_off(60);
        assert_eq!(s.cooldown, RING_DOWN_CALLS);
        for _ in 0..(2 * RING_DOWN_CALLS as usize + 4) {
            s.dsp_sample();
        }
        assert!(!s.active());
    }

    #[test]
    fn fine_tune_retunes_the_divider_once() {
        let mut s = FarfisaKeys::new(&crate::organ::farfisa_fw::synthetic_image());
        s.note_on(60, 60, 1.0);
        s.process(48_000.0, 25.0);
        assert_eq!(s.tuned, 25.0);
    }
}
