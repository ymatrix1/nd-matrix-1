//! Per-wave P table of the strip chain: one EMPIRICAL [`PBlock`] per oscillator stage.
//!
//! The firmware stages are exact for any P block, but the block a loaded program uses is not
//! reachable (`docs/research/synth-wave-models.md` s8 to s10), so the level, the brightness
//! (cutoff factor) and, for SHAPE, the second oscillator are fitted per stage to hardware clips:
//! `docs/research/synth-validation.md` (fit clips, hold-out clips, residuals), driver
//! `tools/synth_validate.py`. Spec: `docs/specs/synth-engine.md` section 4.2.
//!
//! Selection: [`PWave::from_category_wave`] maps the decoded ANALOG `(category, wave)` pair to a
//! row, [`PTable::block`] returns the block, and `SynthLayer::p_table` switches the table on (the
//! block then also scales the stage oscillators and the legacy waveforms that stand in for stages
//! without a Rust model).

use super::p_block::PBlock;

/// Number of rows of the table.
pub const PWAVE_COUNT: usize = 13;

/// One row of the table: the oscillator stage family a decoded ANALOG wave plays through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PWave {
    /// PURE wave 2, saw (`0x1215FB`).
    PureSaw = 0,
    /// PURE waves 3 to 5, square / pulse (`0x121B99`).
    PurePulse,
    /// SHAPE wave 0 (`0x1212D3`).
    ShapeTriangle,
    /// SHAPE wave 1 (`0x12169D`).
    ShapeSaw,
    /// SHAPE wave 2 (`0x121AB5`).
    ShapeSquare,
    /// SHAPE SINE, waves 0 to 5.
    ShapeSine,
    /// MULTI, all waves (`0x121C54`).
    Multi,
    /// SUPER wave 0 (`0x121D6D`).
    Super,
    /// SUPER waves 1 to 4 (stages `Super1`, `Super24`, D22 follow-up 5).
    Super1,
    /// PURE wave 6, white noise (stage `0x121FD8`).
    Noise,
    /// Everything else of the ANALOG bank (sine, triangle, SUB OSC, SYNC, MISC): idle block.
    Other,
    /// The FM oscillator mode (any wave): idle block, own row since D140 so the oracle level
    /// (`docs/research/synth-osc-gain-oracle.md`) can move it without touching the sample path.
    Fm,
    /// The SAMPLE and WAVETABLE oscillator modes: idle block, never measured against the oracle
    /// (the SMP core is not on the ORG stream).
    Sampled,
}

impl PWave {
    const NAMES: [&'static str; PWAVE_COUNT] = [
        "saw",
        "pulse",
        "shape",
        "shapesaw",
        "shapesq",
        "shapesine",
        "multi",
        "super",
        "super1",
        "noise",
        "other",
        "fm",
        "sampled",
    ];

    /// Row of an ANALOG `(category, wave)` pair.
    pub fn from_category_wave(category: u8, wave: u8) -> Self {
        match (category, wave) {
            (0, 2) => Self::PureSaw,
            (0, 3..=5) => Self::PurePulse,
            (3, 0) => Self::ShapeTriangle,
            (3, 1) => Self::ShapeSaw,
            (3, 2) => Self::ShapeSquare,
            // only the waves with a stage model: the others play a legacy waveform whose level
            // the row was not fitted for
            (4, 0 | 1 | 2 | 5) => Self::ShapeSine,
            (5, _) => Self::Multi,
            (6, 0) => Self::Super,
            (6, 1..=4) => Self::Super1,
            (0, 6) => Self::Noise,
            _ => Self::Other,
        }
    }

    /// CLI name (`saw`, `pulse`, `shape`, ...).
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    /// Row by CLI name.
    pub fn from_name(n: &str) -> Option<Self> {
        let i = Self::NAMES.iter().position(|x| x.eq_ignore_ascii_case(n))?;
        Some(Self::ALL[i])
    }

    /// Every row in table order.
    pub const ALL: [PWave; PWAVE_COUNT] = [
        Self::PureSaw,
        Self::PurePulse,
        Self::ShapeTriangle,
        Self::ShapeSaw,
        Self::ShapeSquare,
        Self::ShapeSine,
        Self::Multi,
        Self::Super,
        Self::Super1,
        Self::Noise,
        Self::Other,
        Self::Fm,
        Self::Sampled,
    ];
}

/// One [`PBlock`] per [`PWave`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PTable {
    blocks: [PBlock; PWAVE_COUNT],
    /// The unison stages (MULTI, SUPER) read the verified ARM words (`p_stage_words.rs`) instead of
    /// the ASSUMED constants (default since D81: the ASSUMED constants made a deep-bass SUPER
    /// patch 15 dB too dark above 500 Hz, `Horizon Whl`, `docs/research/synth-validation.md`
    /// s12). The rows of those waves carry an EMPIRICAL level and cutoff factor for the words.
    pub words: bool,
}

impl PTable {
    /// The EMPIRICAL table, `docs/research/synth-validation.md` section 4.
    pub const FITTED: PTable = PTable {
        blocks: BLOCKS,
        words: true,
    };

    /// Block of a row.
    #[inline]
    pub fn block(&self, w: PWave) -> PBlock {
        self.blocks[w as usize]
    }

    /// The table with one row replaced.
    pub fn with(mut self, w: PWave, b: PBlock) -> PTable {
        self.blocks[w as usize] = b;
        self
    }
}

/// Table selection of `--synth-p table[:wave=g/w2/r2/shape/cutoff;...]`: the built-in table plus
/// optional rows in thousandths (fit sweeps). `Eq` so it can live in `FirmwareOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PTableSpec {
    rows: [Option<[i32; 5]>; PWAVE_COUNT],
    words: bool,
}

impl PTableSpec {
    /// The built-in table, no overrides.
    pub const BUILTIN: PTableSpec = PTableSpec {
        rows: [None; PWAVE_COUNT],
        words: true,
    };

    /// Selection from explicit rows (`p_words.rs`).
    pub(super) const fn from_rows(
        rows: [Option<[i32; 5]>; PWAVE_COUNT],
        words: bool,
    ) -> PTableSpec {
        PTableSpec { rows, words }
    }

    /// Parses the part after `table` (empty or `:wave=g/w2/r2/shape/cutoff;...`).
    pub fn parse(rest: &str) -> Result<PTableSpec, String> {
        Self::parse_over(Self::BUILTIN, rest)
    }

    /// Like [`PTableSpec::parse`], the rows overriding `base` (`verified:wave=...` fit sweeps).
    pub fn parse_over(base: PTableSpec, rest: &str) -> Result<PTableSpec, String> {
        let bad = |what: &str| format!("invalid --synth-p table '{rest}': {what}");
        let mut spec = base;
        let body = rest.strip_prefix(':').unwrap_or(rest);
        for item in body.split(';').filter(|s| !s.trim().is_empty()) {
            let (name, vals) = item
                .split_once('=')
                .ok_or_else(|| bad("expected wave=..."))?;
            let w = PWave::from_name(name.trim()).ok_or_else(|| bad("unknown wave"))?;
            let mut m = [0i32; 5];
            let mut it = vals.split('/');
            for slot in &mut m {
                let x: f32 = it
                    .next()
                    .ok_or_else(|| bad("need 5 values"))?
                    .trim()
                    .parse()
                    .map_err(|_| bad("not a number"))?;
                *slot = (x * 1000.0).round() as i32;
            }
            if it.next().is_some() {
                return Err(bad("more than 5 values"));
            }
            spec.rows[w as usize] = Some(m);
        }
        Ok(spec)
    }

    /// The table this selection stands for.
    pub fn table(&self) -> PTable {
        let mut t = PTable::FITTED;
        t.words = self.words;
        for w in PWave::ALL {
            if let Some(m) = self.rows[w as usize] {
                t = t.with(w, PBlock::from_milli(m));
            }
        }
        t
    }
}

/// One row without a second oscillator (`weight2` 0); `shape` scales the Osc Ctrl knob on the
/// SHAPE stage (1.0 = the word equals the knob) and is unused elsewhere.
const fn row(gain: f32, shape: f32, cutoff_scale: f32) -> PBlock {
    PBlock {
        gain,
        weight2: 0.0,
        ratio2: 1.0,
        shape,
        cutoff_scale,
    }
}

/// Pure Saw row. EMPIRICAL: gain 0.4 (RMS delta +0.4 dB on `filter_bank`, A:75, band pass,
/// velocity 100; `phat_3osc` +1.0 dB hold-out) and cutoff factor 0.85 from the laser-zap saw
/// (LP24, best 0.8 to 0.9). The earlier disagreement with `filter_bank` (best 1.25 as a saw
/// factor) is carried by the band pass type factor of [`type_cutoff_scale`] instead.
/// `docs/research/synth-validation.md` sections 4 and 7.
const SAW: PBlock = row(0.4, 0.5, 0.85);
/// Pure Square row. EMPIRICAL: gain = saw gain x 0.68 (the two notes of the pitch-blip clip
/// differ by 3.4 dB, so the pulse sits 3.4 dB under the saw; one pair, velocity unknown); the
/// cutoff factor is the saw's (ASSUMED, no filtered pulse clip).
const PULSE: PBlock = row(0.27, 0.5, 0.85);
/// SHAPE wave 0 row. EMPIRICAL: gain 0.6 and cutoff factor 1.75 from the Smooth Lead clip (A:87,
/// fit; 1.0 gives presence -10.1 dB, 1.35 -3.7, 1.75 -2.0), no second oscillator, the shape word
/// equal to the Osc Ctrl knob (`shape` 1.0 scales it); hold-out Crunch Synth (A:66, score 8.3
/// legacy to 1.2).
const SHAPE: PBlock = row(0.6, 1.0, 1.75);
/// MULTI row (D81: words stage; gain 3.8 and factor 2.0 refit on `sci_fi_80s`, `mega_line`,
/// `horizon_whl`, RMS +3.0 / -5.2 / +1.6 dB; the numbers below are the pre-D81 ASSUMED-stage fit).
/// Before D81: EMPIRICAL gain 0.75 and cutoff factor 1.4 from `sci_fi_80s` (A:44, fit; the
/// gain was 0.26 until the Ladder24 stage of PR 319 replaced the filter that clip uses, then the
/// clip read -10.4 dB RMS: gains 0.26 / 0.7 / 0.9 give RMS -10.4 / -1.3 / +1.2 dB, score 1.39 /
/// 0.82 / 0.99, with the Ladder24 type factor 0.8 of [`super::p_type`]); `mega_line` (A:48)
/// gains from it too (RMS -3.2 to -2.5 dB).
const MULTI: PBlock = row(3.8, 0.5, 2.0);
/// SUPER wave 0 row (D81: the stage reads the verified ARM words, [`PTable::words`]).
/// EMPIRICAL: gain 2.4 = 1.5 x the verified level constant 1.6 (RMS -1.0 dB on `pad_silk`, A:86,
/// +0.1 dB on `horizon_whl`, A:16; 1.6 reads -4 dB on the pad) and cutoff factor 0.45 (LP12:
/// 0.45 x 1.4 = 0.63, [`super::p_type::row_type_cutoff_scale`] gives LP24 its own total 2.0, the
/// `horizon_whl` fit: LP24 277 / 554 Hz, score 10.2 to 1.5). Before D81: gain 2.0 (RMS +0.2 dB on
/// `pad_silk`) with the ASSUMED stage constants and cutoff factor
/// 0.45, times the LP12 type factor 1.4 = 0.63: grid on `pad_silk` over the row factor 0.25 /
/// 0.36 / 0.42 / 0.45 / 0.5 / 0.7 gives score 4.69 / 1.97 / 1.88 / 2.36 / 3.08 / 5.24 (was 7.62
/// with the old compromise 1.07; 0.45 keeps `mega_line` inside the bar). The old conflict with `mega_line` (that clip wants 3.1 in total) came from
/// one row serving SUPER waves 0 and 1: wave 1 now has its own row, [`SUPER1`]. The envelope
/// octave hypothesis of the earlier section 7 is not needed. Disclosure: `pad_silk` is the fit
/// clip; the three Super Saw lesson clips (filter open) do not act on the factor.
const SUPER: PBlock = row(2.4, 0.5, 0.45);
/// SUPER waves 1 to 4 row (D81: words stage, gain 1.6 (verified level) and factor 2.2, `mega_line`;
/// before D81 gain 1.0 and factor 3.2) (stages `Super1`, `Super24`, which play after PR 292 instead of the
/// legacy saw). EMPIRICAL: gain 1.0, cutoff factor 3.2 from `mega_line` (A:48, LP12 1200 Hz,
/// wave 1 plus a SUPER wave 0 oscillator; with the SUPER row at 1.07 the grid factor 0.7 / 1.0
/// / 1.5 / 2.2 gives score 2.74 / 2.53 / 2.10 / 1.58 at gain 1, with the SUPER row at 0.45 the
/// factor 2.6 / 3.2 gives 1.91 / 1.69; gain 1.6 is worse, 3.29 at factor 1.0). The clip was a
/// hold-out before; waves 2 to 4 reuse the row (ASSUMED, no clip).
const SUPER1: PBlock = row(1.6, 0.5, 2.2);
/// SHAPE SINE row (waves 0 to 2, 5). EMPIRICAL: gain 0.5 and cutoff factor 3.0 from `sine_pluck`
/// (A:26, LP12 185 Hz, one clip, no hold-out): gain 0.5 = RMS -1.0 dB (idle 1.0 reads +5.3);
/// factor 1.0 / 2.0 / 3.0 / 4.0 gives presence -18.6 / -14.6 / -12.2 / -11.3 dB, score 6.52 /
/// 4.73 / 4.75 / 4.83. The remaining presence gap is the SHAPE SINE word, not the filter.
const SHAPE_SINE: PBlock = row(0.5, 1.0, 3.0);

/// PURE wave 6 (white noise) row. The ARM writes `P[16] = 0.25` for noise, exactly as for the PURE
/// saw (`docs/research/synth-p-writer.md` section 3, VERIFIED run), and the stage output peaks at
/// the same +-1, so the row takes the saw gain 0.4 (EMPIRICAL, [`SAW`]) instead of the idle 1.0
/// that sat 8 dB too loud on `Leaky Stabs II` (A:46, a white-noise layer, `synth_leaky`). The
/// cutoff factor stays 1.0 (ASSUMED, no clip of a filtered noise alone).
/// `docs/research/synth-validation.md` section 14.
const NOISE: PBlock = row(0.4, 0.5, 1.0);

/// Rows in [`PWave`] order. SHAPE saw reuses the saw row and SHAPE SQUARE / SHAPE SINE / other
/// waves stay at the idle block: ASSUMED, no fit clip with a hold-out.
const BLOCKS: [PBlock; PWAVE_COUNT] = [
    SAW,
    PULSE,
    SHAPE,
    SAW,
    PBlock::IDLE,
    SHAPE_SINE,
    MULTI,
    SUPER,
    SUPER1,
    NOISE,
    PBlock::IDLE,
    PBlock::IDLE,
    PBlock::IDLE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_wave_pairs_map_to_their_stage_rows() {
        assert_eq!(PWave::from_category_wave(0, 2), PWave::PureSaw);
        assert_eq!(PWave::from_category_wave(0, 4), PWave::PurePulse);
        assert_eq!(PWave::from_category_wave(3, 0), PWave::ShapeTriangle);
        assert_eq!(PWave::from_category_wave(3, 1), PWave::ShapeSaw);
        assert_eq!(PWave::from_category_wave(3, 2), PWave::ShapeSquare);
        assert_eq!(PWave::from_category_wave(4, 5), PWave::ShapeSine);
        assert_eq!(
            PWave::from_category_wave(4, 3),
            PWave::Other,
            "no stage model"
        );
        assert_eq!(PWave::from_category_wave(5, 3), PWave::Multi);
        assert_eq!(PWave::from_category_wave(6, 0), PWave::Super);
        assert_eq!(PWave::from_category_wave(6, 1), PWave::Super1);
        assert_eq!(PWave::from_category_wave(6, 4), PWave::Super1);
        assert_eq!(PWave::from_category_wave(6, 5), PWave::Other);
        assert_eq!(PWave::from_category_wave(0, 6), PWave::Noise);
        assert_eq!(PWave::from_category_wave(0, 0), PWave::Other);
        assert_eq!(PWave::from_category_wave(7, 0), PWave::Other);
    }

    #[test]
    fn super_rows_are_separate_for_wave_zero_and_the_stage_waves() {
        let w0 = PTable::FITTED.block(PWave::Super).cutoff_scale;
        let w1 = PTable::FITTED.block(PWave::Super1).cutoff_scale;
        assert!(w1 > 3.0 * w0, "wave 0 {w0}, wave 1 {w1}");
        assert_eq!(PTable::FITTED.block(PWave::PureSaw).cutoff_scale, 0.85);
        assert_eq!(PTable::FITTED.block(PWave::PurePulse).cutoff_scale, 0.85);
        assert_eq!(PTable::FITTED.block(PWave::ShapeSine).gain, 0.5);
    }

    #[test]
    fn the_default_table_reads_the_verified_words() {
        // D81: the ASSUMED unison constants left Horizon Whl 15 dB dark above 500 Hz
        let fitted = PTable::FITTED;
        assert!(fitted.words);
        assert!(PTableSpec::BUILTIN.table().words);
        assert!(PTableSpec::parse("").unwrap().table().words);
        assert_eq!(PTable::FITTED.block(PWave::Super).gain, 2.4);
    }

    #[test]
    fn noise_row_takes_the_saw_gain_like_the_arm_words() {
        // VERIFIED words: P[16] = 0.25 for PURE noise and PURE saw (synth-p-writer s3).
        let noise = PTable::FITTED.block(PWave::Noise);
        let saw = PTable::FITTED.block(PWave::PureSaw);
        assert_eq!(noise.gain, saw.gain);
        assert_eq!(noise.cutoff_scale, 1.0);
        assert_eq!(PWave::from_name("noise"), Some(PWave::Noise));
        // the stage output of a noise layer is scaled by the row against the idle block
        let st = Some(super::super::FwStage::Noise);
        let off = rms(None, PWave::Noise, st);
        let on = rms(Some(PTable::FITTED), PWave::Noise, st);
        assert!(off > 1e-4, "{off}");
        assert!((on / off - noise.gain).abs() < 0.02, "{on} vs {off}");
    }

    #[test]
    fn names_round_trip_and_cover_every_row() {
        for w in PWave::ALL {
            assert_eq!(PWave::from_name(w.name()), Some(w));
        }
        assert_eq!(PWave::from_name("SAW"), Some(PWave::PureSaw));
        assert_eq!(PWave::from_name("nope"), None);
        assert_eq!(PWave::ALL.len(), PWAVE_COUNT);
    }

    #[test]
    fn spec_overrides_one_row_and_keeps_the_rest() {
        let s = PTableSpec::parse(":saw=0.5/0/1/0.5/1.2;super=0.7/0/1/0.5/0.9").unwrap();
        let t = s.table();
        assert_eq!(t.block(PWave::PureSaw).gain, 0.5);
        assert_eq!(t.block(PWave::PureSaw).cutoff_scale, 1.2);
        assert_eq!(t.block(PWave::Super).gain, 0.7);
        assert_eq!(
            t.block(PWave::Multi),
            PTable::FITTED.block(PWave::Multi),
            "untouched rows keep the built-in values"
        );
        assert_eq!(PTableSpec::parse("").unwrap().table(), PTable::FITTED);
    }

    #[test]
    fn spec_rejects_malformed_rows() {
        for bad in [
            ":saw",
            ":nope=1/1/1/1/1",
            ":saw=1/2/3/4",
            ":saw=1/2/3/4/5/6",
            ":saw=a/b/c/d/e",
        ] {
            assert!(PTableSpec::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn built_in_rows_are_inside_the_physical_ranges() {
        for w in PWave::ALL {
            let p = PTable::FITTED.block(w);
            assert!(p.gain > 0.0 && p.gain < 8.0, "{w:?} gain");
            assert!((0.0..=4.0).contains(&p.weight2), "{w:?} weight2");
            assert!((0.05..=8.0).contains(&p.ratio2), "{w:?} ratio2");
            assert!((-1.0..=1.0).contains(&p.shape), "{w:?} shape");
            assert!((0.25..=4.0).contains(&p.cutoff_scale), "{w:?} cutoff");
        }
    }

    fn rms(table: Option<PTable>, wave: PWave, stage: Option<super::super::FwStage>) -> f32 {
        use super::super::{SynthChain, SynthLayer, VaWaveform};
        let mut s = SynthLayer::new(48_000.0);
        s.chain = SynthChain::FirmwareStrip;
        s.filter_on = false;
        s.waveform = VaWaveform::Saw;
        s.p_table = table;
        s.p_wave = wave;
        s.strip_stage = stage;
        s.strip_stages = stage.is_some();
        s.note_on(60, 127);
        let mut acc = 0.0f64;
        for i in 0..24_000 {
            let [l, _] = s.render_frame();
            if i >= 12_000 {
                acc += f64::from(l * l);
            }
        }
        (acc / 12_000.0).sqrt() as f32
    }

    #[test]
    fn table_gain_scales_the_saw_and_the_stage_oscillators() {
        let half = |w| {
            PTable::FITTED.with(
                w,
                PBlock {
                    gain: 0.5,
                    ..PBlock::IDLE
                },
            )
        };
        let saw0 = rms(Some(PTable::FITTED), PWave::PureSaw, None);
        let saw1 = rms(Some(half(PWave::PureSaw)), PWave::PureSaw, None);
        let g_saw = PTable::FITTED.block(PWave::PureSaw).gain;
        assert!(saw0 > 1e-4, "{saw0}");
        assert!((saw1 / saw0 - 0.5 / g_saw).abs() < 2e-3, "{saw1} vs {saw0}");
        let st = Some(super::super::FwStage::Super);
        let sup0 = rms(Some(PTable::FITTED), PWave::Super, st);
        let sup1 = rms(Some(half(PWave::Super)), PWave::Super, st);
        let g_sup = PTable::FITTED.block(PWave::Super).gain;
        assert!(sup0 > 1e-4, "{sup0}");
        assert!((sup1 / sup0 - 0.5 / g_sup).abs() < 2e-3, "{sup1} vs {sup0}");
    }

    #[test]
    fn rows_of_other_waves_do_not_touch_a_layer() {
        let loud = PTable::FITTED.with(
            PWave::Multi,
            PBlock {
                gain: 4.0,
                ..PBlock::IDLE
            },
        );
        let a = rms(Some(PTable::FITTED), PWave::PureSaw, None);
        let b = rms(Some(loud), PWave::PureSaw, None);
        assert_eq!(a, b);
        let off = rms(None, PWave::Other, None);
        let idle = rms(Some(PTable::FITTED), PWave::Other, None);
        assert!(
            (off / idle - 1.0).abs() < 1e-3,
            "idle row equals no table: {off} vs {idle}"
        );
        assert!(
            (a / off - 0.4).abs() < 0.02,
            "the saw row scales by its gain: {a} vs {off}"
        );
    }
}
