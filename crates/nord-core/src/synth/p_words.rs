//! The oscillator P block words the ARM writes per wave type (VERIFIED by run), and the opt-in
//! `--synth-p verified` table built from them.
//!
//! The ARM strip apply function `0xC01CE7D8` calls the table driven writers `0xC01C72CC`,
//! `0xC01C7550` and `0xC01C94CC`, which send ~44 one-word messages `(slot, float)` into
//! `DM(0xC169A + 0x96 slot + index)` of the ORG / PNO core 2 images (`tools/synth_p_writer.py`,
//! `docs/research/synth-p-writer.md`). The capture `docs/research/data/synth-p-words.csv` holds
//! the words of slot 0 per type index (`a = b = 0` rows) plus the unison variants. Index
//! convention: `p_index = dm - 0xC169A` (the stage reads `P[-32 .. 117]`).
//!
//! What the words determine: gain `P[16 + lane]`, own / cross weights `P[-16 .. -9]`, the two
//! oscillator ratios `P[-32 + lane]`, `P[-30 + lane]` and, for the SUPER stages, the sixteen
//! unison ratios `P[-32 .. -17]`. What they do not: the level of the whole chain (the voice gain
//! chain after the stage) and the cutoff factors, which stay EMPIRICAL.
//!
//! Spec: `docs/specs/synth-engine.md` section 4.2.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::p_table::{PTable, PTableSpec, PWave, PWAVE_COUNT};

const WORDS_CSV: &str = include_str!("../../../../docs/research/data/synth-p-words.csv");
const TYPES_CSV: &str = include_str!("../../../../docs/research/data/synth-wave-types.csv");

/// EMPIRICAL: level of the voice chain behind the stage in units of `P[16]` (the saw row of the
/// EMPIRICAL table has gain 0.4 for `P[16] = 0.25`, so one unit of `P[16]` is 1.6 engine gain;
/// the Pure Saw level is the one the hardware clips fix best, `docs/research/synth-p-fit.md`
/// s2: one oscillator, 0.3 dB harmonic residual). The pulse rows then follow from the words
/// alone: `1.6 x 0.175 = 0.28` against the fitted 0.27.
pub const GAIN_PER_P16: f32 = 1.6;

/// Words of one type, `p_index -> bits`.
#[derive(Debug, Clone, Default)]
pub struct PWords {
    words: BTreeMap<i32, u32>,
}

impl PWords {
    /// Float word at `P[index]`, if the ARM wrote it for this type.
    pub fn get(&self, index: i32) -> Option<f32> {
        self.words.get(&index).map(|b| f32::from_bits(*b))
    }

    fn at(&self, index: i32) -> f32 {
        self.get(index).unwrap_or(0.0)
    }

    /// Stage output gain `P[16]` (lane x).
    pub fn gain(&self) -> f32 {
        self.at(16)
    }

    /// Weight of the first oscillator as seen at lane x output (own `P[-16]` plus cross
    /// `P[-13]`: the lane mix of `0x242742..0x2428BA`, both lanes carry the same wave).
    pub fn osc1_weight(&self) -> f32 {
        self.at(-16) + self.at(-13)
    }

    /// Weight of the second oscillator at lane x output (own `P[-12]` plus cross `P[-9]`).
    pub fn osc2_weight(&self) -> f32 {
        self.at(-12) + self.at(-9)
    }

    /// Second oscillator frequency relative to the first, lane x (`P[-30] / P[-32]`).
    pub fn ratio2(&self) -> f32 {
        let r1 = self.at(-32);
        if r1 == 0.0 {
            1.0
        } else {
            self.at(-30) / r1
        }
    }

    /// The unison voice ratios of lane x (`P[-32 + 2 j]`, `j = 0 .. 7`), when the type writes the
    /// sixteen-word spread table (the SUPER types 22 to 26).
    pub fn unison_ratios_x(&self) -> Option<[f32; 8]> {
        if !self.words.contains_key(&-17) {
            return None;
        }
        let mut out = [0.0; 8];
        for (j, r) in out.iter_mut().enumerate() {
            *r = self.get(-32 + 2 * j as i32)?;
        }
        Some(out)
    }

    /// Engine gain of a single-oscillator stage: `GAIN_PER_P16 * P[16] * osc1 weight`.
    pub fn engine_gain(&self) -> f32 {
        GAIN_PER_P16 * self.gain() * self.osc1_weight()
    }
}

struct Tables {
    by_type: Vec<PWords>,
    /// `(category, wave) -> type`, bank 0 (ANALOG).
    type_of: BTreeMap<(u8, u8), usize>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut by_type: Vec<PWords> = Vec::new();
        for line in WORDS_CSV.lines().skip(1) {
            let f: Vec<&str> = line.split(',').collect();
            if f.len() < 7 || f[1] != "0" || f[2] != "0" {
                continue;
            }
            let (Ok(t), Ok(i), Some(bits)) = (
                f[0].parse::<usize>(),
                f[4].parse::<i32>(),
                f[5].strip_prefix("0x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok()),
            ) else {
                continue;
            };
            if by_type.len() <= t {
                by_type.resize_with(t + 1, PWords::default);
            }
            by_type[t].words.insert(i, bits);
        }
        let mut type_of = BTreeMap::new();
        for line in TYPES_CSV.lines().skip(1) {
            let f: Vec<&str> = line.split(',').collect();
            if f.len() < 5 || f[1] != "0" {
                continue;
            }
            if let (Ok(c), Ok(w), Ok(t)) = (f[2].parse(), f[3].parse(), f[4].parse()) {
                type_of.insert((c, w), t);
            }
        }
        Tables { by_type, type_of }
    })
}

/// Type index of an ANALOG `(category, wave)` pair (`data/synth-wave-types.csv`).
pub fn type_of(category: u8, wave: u8) -> Option<usize> {
    tables().type_of.get(&(category, wave)).copied()
}

/// Number of types in the capture.
pub fn type_count() -> usize {
    tables().by_type.len()
}

/// The words of a type.
pub fn words_of_type(t: usize) -> Option<&'static PWords> {
    tables().by_type.get(t).filter(|w| !w.words.is_empty())
}

/// The words of an ANALOG `(category, wave)` pair.
pub fn words_for(category: u8, wave: u8) -> Option<&'static PWords> {
    words_of_type(type_of(category, wave)?)
}

/// Representative `(category, wave)` of each table row that a single oscillator describes.
const ROWS: [(PWave, u8, u8); 6] = [
    (PWave::PureSaw, 0, 2),
    (PWave::PurePulse, 0, 3),
    (PWave::ShapeTriangle, 3, 0),
    (PWave::ShapeSaw, 3, 1),
    (PWave::ShapeSquare, 3, 2),
    (PWave::ShapeSine, 4, 0),
];

impl PTableSpec {
    /// The table of `--synth-p verified`: for the rows a single oscillator describes, the gain is
    /// `GAIN_PER_P16 * P[16] * weight` from the ARM words (VERIFIED words, one EMPIRICAL level
    /// constant); the MULTI / SUPER stages read the words themselves (`PTable::words`) and their
    /// rows carry `GAIN_PER_P16`. The cutoff factors are the EMPIRICAL ones of
    /// [`PTable::FITTED`] (`docs/research/synth-validation.md` s11).
    pub fn verified() -> PTableSpec {
        let mut rows = [None; PWAVE_COUNT];
        for (w, c, v) in ROWS {
            if let Some(words) = words_for(c, v) {
                let b = PTable::FITTED.block(w);
                rows[w as usize] = Some([
                    (words.engine_gain() * 1000.0).round() as i32,
                    0,
                    1000,
                    (b.shape * 1000.0).round() as i32,
                    (b.cutoff_scale * 1000.0).round() as i32,
                ]);
            }
        }
        // The unison rows: the stage reads the words itself (`p_stage_words.rs`), so the row
        // carries the engine level only.
        for w in [PWave::Multi, PWave::Super, PWave::Super1] {
            let b = PTable::FITTED.block(w);
            rows[w as usize] = Some([
                (GAIN_PER_P16 * 1000.0).round() as i32,
                0,
                1000,
                (b.shape * 1000.0).round() as i32,
                (b.cutoff_scale * 1000.0).round() as i32,
            ]);
        }
        PTableSpec::from_rows(rows, true)
    }

    /// The table of `--synth-p oracle` (D140): the built-in table with the three rows that were
    /// never fitted (ASSUMED idle block, gain 1.0) at `GAIN_PER_P16 * 0.25 = 0.4`: SHAPE square,
    /// the ANALOG `Other` row (PURE sine / triangle, SUB OSC, SYNC, MISC) and the FM row.
    ///
    /// VERIFIED (oracle, `tools/synth_osc_gain.py`, `docs/research/data/synth-osc-gain-oracle.csv`):
    /// the native oscillator times the fixed voice constant 0.25 equals the ORG core 2 oscillator
    /// stage at row gain 1.0 within 0.1 dB (needed row gain 1.006, 1.006, 1.006 on three SHAPE
    /// square programs, 1.001 on PURE sine, 0.999 / 1.003 on two FM programs, 0.956 on a third with an octave mismatch), and the ARM writes
    /// `P[16] = 0.25` for these waves like for the saw, whose hardware fit is 0.4: the oracle puts
    /// all of them at the same level, so an idle 1.0 sits +8 dB against the saw. The link to the
    /// saw anchor (the voice gain behind the stage, 0.4) is EMPIRICAL (the oracle stream has no
    /// output stage). SYNC pulse types write 0.175 / 0.2 (`synth-p-words.csv` types 30, 31): they
    /// share the `Other` row and sit about 3 / 2 dB high (ASSUMED, no played SYNC program in the
    /// measured set). SAMPLE and WAVETABLE rows stay idle: not on the oracle stream.
    pub fn oracle() -> PTableSpec {
        let row = format!("{:.3}/0/1/0.5/1", GAIN_PER_P16 * 0.25);
        let rest = format!(":shapesq={row};other={row};fm={row}");
        Self::parse(&rest).expect("fixed oracle rows parse")
    }

    /// The table of `--synth-p coupled` (D194): [`PTableSpec::oracle`] plus the SHAPE triangle,
    /// MULTI and SUPER rows at their oracle level times the one coupling factor
    /// [`COUPLED_FILTER_GAIN`]; the cutoff factors, shape scale and every other row stay as the
    /// built-in table (EMPIRICAL).
    ///
    /// VERIFIED (oracle, `tools/synth_osc_gain.py`, `docs/research/data/synth-osc-gain-oracle.csv`):
    /// the oscillator row that puts native on the ORG core 2 stage is `0.4 x 1.000` (SHAPE
    /// triangle, 3 programs), `0.4 x 4.047` = 1.62 (MULTI, 3 programs, the verified 1.6) and
    /// `0.4 x 4.755` = 1.90 (SUPER wave 0, 2 programs: [`ORACLE_SUPER_ROW`]). The fitted rows
    /// 0.6 / 3.8 / 2.4 sit 3.5 / 7.4 / 2.0 dB above those; the oracle filter reads 2.7 to 6.7 dB
    /// above the native one on 14 of 18 programs (`docs/research/data/synth-filter-gain-oracle.csv`),
    /// so the three fitted gains are one filter gain times the oracle row, not three free levels.
    pub fn coupled() -> PTableSpec {
        Self::coupled_with(COUPLED_FILTER_GAIN)
    }

    /// The table of `--synth-p oracle-rows` (D195): the oracle rows alone (`g = 1`), for use with
    /// the native filter gain by type (`--synth-filter-gain`) instead of the coupling factor.
    pub fn oracle_rows() -> PTableSpec {
        Self::coupled_with(1.0)
    }

    fn coupled_with(g: f32) -> PTableSpec {
        let rows = format!(
            ":shape={:.3}/0/1/1/1.75;multi={:.3}/0/1/0.5/2;super={:.3}/0/1/0.5/0.45",
            GAIN_PER_P16 * 0.25 * g,
            GAIN_PER_P16 * g,
            ORACLE_SUPER_ROW * g
        );
        Self::parse_over(Self::oracle(), &rows).expect("fixed coupled rows parse")
    }
}

/// Oracle row of SUPER wave 0: the median needed row gain 4.755 (4.591 and 4.918 on `Glow Chords
/// Whl` and `Horizon Whl`, D140) times `GAIN_PER_P16 * 0.25`. VERIFIED (measurement); the
/// verified words stage reads `GAIN_PER_P16` = 1.6 (1.8 dB lower, the unison words carry their own
/// `P[16]`, `docs/research/synth-osc-gain-oracle.md` s3).
pub const ORACLE_SUPER_ROW: f32 = 1.9;

/// The one factor of `--synth-p coupled`: the three fitted rows are the oracle rows times this
/// native filter gain (`sqrt 2`, +3.0 dB). EMPIRICAL: swept 1.0 / 1.414 / 1.778 / 2.0 on 16 Part A
/// synth clips with `--synth-unison-weights`: mean score 2.693 / 2.706 / 2.733 / 2.743 against 2.747
/// for the fitted rows with the weights, mean absolute RMS error 4.50 / 3.81 / 4.25 / 4.48 dB against
/// 3.89 dB (`docs/research/synth-rows-unison.md` s4). The oracle gap that this stands for is
/// +4.6 dB (LP12, 5 programs without the resonance 0.95 outlier) to +5.3 dB (LP24, 7 programs), measured per program in s3.
pub const COUPLED_FILTER_GAIN: f32 = std::f32::consts::SQRT_2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saw_is_one_oscillator_with_the_arm_gain() {
        let w = words_for(0, 2).unwrap();
        assert_eq!(type_of(0, 2), Some(2));
        assert_eq!((w.at(-32), w.at(-30), w.gain()), (1.0, 1.0, 0.25));
        assert_eq!((w.osc1_weight(), w.osc2_weight()), (1.0, 0.0));
        assert!((w.engine_gain() - 0.4).abs() < 1e-6, "calibrates the level");
    }

    #[test]
    fn pulse_types_differ_by_the_gain_only() {
        let g: Vec<f32> = (3..=5).map(|v| words_for(0, v).unwrap().gain()).collect();
        assert_eq!(g, vec![0.175, 0.2, 0.25]);
        let square = words_for(0, 3).unwrap().engine_gain();
        assert!((square - 0.28).abs() < 1e-6, "fitted 0.27: {square}");
    }

    #[test]
    fn multi_has_a_detuned_second_oscillator() {
        let w = words_for(5, 0).unwrap();
        assert_eq!(type_of(5, 0), Some(16));
        assert!((w.ratio2() - 1.008_702).abs() < 1e-6);
        assert!((w.osc2_weight() - 0.85).abs() < 1e-6);
        assert!((w.gain() - 0.176_776_7).abs() < 1e-6);
    }

    #[test]
    fn super_types_carry_the_unison_ratios() {
        let w = words_for(6, 2).unwrap();
        let r = w.unison_ratios_x().unwrap();
        assert!((r[0] - 0.997_116).abs() < 1e-6 && (r[1] - 1.0087).abs() < 1e-4);
        assert!((r[7] - 1.04427).abs() < 1e-5);
        assert_eq!(w.gain(), 0.125);
        assert!((w.at(-16) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!(words_for(0, 2).unwrap().unison_ratios_x().is_none());
        assert_eq!(words_for(6, 0).unwrap().unison_ratios_x(), Some(r));
    }

    #[test]
    fn every_analog_wave_has_words() {
        assert_eq!(type_count(), 74);
        for (c, n) in [7u8, 3, 9, 5, 7, 6, 5, 3].iter().enumerate() {
            for v in 0..*n {
                assert!(words_for(c as u8, v).is_some(), "({c},{v})");
            }
        }
    }

    #[test]
    fn verified_is_a_synth_p_selection() {
        use crate::synth::PSpec;
        let spec = PSpec::parse("verified").unwrap();
        assert_eq!(spec, PSpec::Verified);
        assert_eq!(spec.table(), Some(PTableSpec::verified().table()));
        assert_ne!(spec, PSpec::Table(PTableSpec::BUILTIN));
        assert!(PSpec::parse("VERIFIED").is_ok());
    }

    #[test]
    fn verified_table_changes_gains_only() {
        let t = PTableSpec::verified().table();
        let f = PTable::FITTED;
        assert_eq!(t.block(PWave::PureSaw), f.block(PWave::PureSaw));
        let p = t.block(PWave::PurePulse);
        assert!((p.gain - 0.28).abs() < 1e-6);
        assert_eq!(p.cutoff_scale, f.block(PWave::PurePulse).cutoff_scale);
        assert!(
            t.words && f.words,
            "the words stages are the default since D81"
        );
        // the unison rows carry the engine level only, the cutoff factors stay
        for w in [PWave::Multi, PWave::Super, PWave::Super1] {
            let (b, o) = (t.block(w), f.block(w));
            assert_eq!(
                (b.gain, b.cutoff_scale),
                (GAIN_PER_P16, o.cutoff_scale),
                "{w:?}"
            );
        }
    }

    #[test]
    fn oracle_moves_only_the_idle_rows_that_the_oracle_measured() {
        use crate::synth::PSpec;
        let (o, f) = (PTableSpec::oracle().table(), PTable::FITTED);
        let want = GAIN_PER_P16 * 0.25;
        for w in [PWave::ShapeSquare, PWave::Other, PWave::Fm] {
            assert!((o.block(w).gain - want).abs() < 1e-6, "{w:?}");
            assert_eq!(f.block(w).gain, 1.0, "the built-in idle row {w:?}");
            assert_eq!(o.block(w).cutoff_scale, f.block(w).cutoff_scale);
        }
        for w in PWave::ALL {
            if !matches!(w, PWave::ShapeSquare | PWave::Other | PWave::Fm) {
                assert_eq!(o.block(w), f.block(w), "{w:?}");
            }
        }
        assert_eq!(
            o.block(PWave::Sampled).gain,
            1.0,
            "samples are not on the stream"
        );
        assert!(o.words, "the words stages stay on");
        assert_eq!(PSpec::parse("Oracle").unwrap(), PSpec::Oracle);
        assert_eq!(PSpec::Oracle.table(), Some(o));
    }

    #[test]
    fn coupled_is_the_oracle_rows_times_one_filter_gain() {
        use crate::synth::PSpec;
        let (c, o, f) = (
            PTableSpec::coupled().table(),
            PTableSpec::oracle().table(),
            PTable::FITTED,
        );
        let g = COUPLED_FILTER_GAIN;
        assert!((g - 1.414).abs() < 1e-3);
        for (w, row) in [
            (PWave::ShapeTriangle, 0.4),
            (PWave::Multi, GAIN_PER_P16),
            (PWave::Super, ORACLE_SUPER_ROW),
        ] {
            let b = c.block(w);
            assert!((b.gain - row * g).abs() < 1.5e-3, "{w:?} {}", b.gain);
            let fit = f.block(w);
            assert_eq!(
                (b.weight2, b.ratio2, b.shape, b.cutoff_scale),
                (fit.weight2, fit.ratio2, fit.shape, fit.cutoff_scale),
                "only the level moves: {w:?}"
            );
        }
        for w in PWave::ALL {
            if !matches!(w, PWave::ShapeTriangle | PWave::Multi | PWave::Super) {
                assert_eq!(c.block(w), o.block(w), "the oracle rows elsewhere: {w:?}");
            }
        }
        assert!(c.words, "the words stages stay on");
        assert_eq!(PSpec::parse("Coupled").unwrap(), PSpec::Coupled);
        assert_eq!(PSpec::Coupled.table(), Some(c));
        assert_ne!(PSpec::Coupled, PSpec::Oracle);
    }

    #[test]
    fn oracle_rows_are_the_coupled_rows_without_the_factor() {
        use crate::synth::PSpec;
        let (r, o) = (
            PTableSpec::oracle_rows().table(),
            PTableSpec::oracle().table(),
        );
        for (w, row) in [
            (PWave::ShapeTriangle, 0.4),
            (PWave::Multi, GAIN_PER_P16),
            (PWave::Super, ORACLE_SUPER_ROW),
        ] {
            assert!(
                (r.block(w).gain - row).abs() < 1.5e-3,
                "{w:?} {}",
                r.block(w).gain
            );
        }
        assert_eq!(r.block(PWave::ShapeSquare), o.block(PWave::ShapeSquare));
        assert_eq!(PSpec::parse("Oracle-Rows").unwrap(), PSpec::OracleRows);
        assert_eq!(PSpec::OracleRows.table(), Some(r));
    }

    #[test]
    fn verified_rows_can_be_overridden_for_fit_sweeps() {
        use crate::synth::PSpec;
        let PSpec::Table(spec) = PSpec::parse("verified:super=1.6/0/1/0.5/0.7").unwrap() else {
            panic!("table")
        };
        let t = spec.table();
        assert!(t.words);
        assert_eq!(t.block(PWave::Super).cutoff_scale, 0.7);
        assert_eq!(t.block(PWave::Multi).gain, GAIN_PER_P16);
        assert!(PSpec::parse("verifiedx").is_err());
    }
}
