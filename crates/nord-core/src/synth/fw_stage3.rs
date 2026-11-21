//! Firmware synth filter stage `0x1257B2` (raw `Edt_Syn_kFltType` 3, "LP+HP"), opt-in.
//!
//! Spec: `docs/specs/synth-engine.md` section 4.1; evidence `docs/research/synth-filter-t3.md`;
//! bit-exact model `tools/synth_filter_t3.py` (1840 of 1840 oracle words over 40 trials);
//! goldens `tests_fw_stage3_golden.rs`.
//!
//! A coupled second order form with five states per lane. The stage builds its own coefficients
//! from the cutoff coefficient `R` (frame `-30`) and the table position (frame `-28`):
//!
//! ```text
//! i, fr = FIX(R/2, 31)                 U = T1 lerp(i, fr)   V = T2 lerp(i, fr)   g = 2 U
//! Wv = T0 lerp(position)               i2, f2 = FIX(Wv/2, 31)
//! X1 = T1 lerp(i2, f2)   X2 = T2 lerp(i2, f2)
//! kc = CLIP(4.5 g + 0.25, 1)    h = 1 - (X1^2 + X1 X2)    kh = kc h
//! ```
//!
//! and per sample (states `a b c d e`, `x1x2 = 2 X1`):
//!
//! ```text
//! p = b + g a     e' = e + x1x2 c     d' = x      b' = p
//! n = (x kh - e') - X2 c        m = ((d + x)/2 - p) - V a
//! y = n + (b + p)/2             c' = CLIP(c + x1x2 n, 1)     a' = CLIP(a + g m, 1)
//! ```

use super::fw_filter::TablePos;
use super::fw_stage3_tables::{T0, T1, T2};

/// VERIFIED: image word `DM(0xC28E6) = 0x40900000`, operand of the listing `0x24AF64..`; the
/// gain of `g` in `kc`. Checked by the oracle (words equal, `tools/synth_filter_t3.py`).
const K_GAIN: f32 = 4.5;
/// VERIFIED: image word `DM(0xC28DA) = 0.25` (offset of `kc`).
const QUARTER: f32 = 0.25;
/// VERIFIED: image word `DM(0xC28DE) = 1.0`, the clip limit of `kc` and of the states `a`, `c`.
const CLIP: f32 = 1.0;
/// Highest table index (the tables have 129 entries).
const IDX_MAX: usize = 127;

#[inline]
fn lerp(table: &[u32; 129], idx: usize, fr: f32) -> f32 {
    let lo = f32::from_bits(table[idx]);
    let hi = f32::from_bits(table[idx + 1]);
    lo + (hi - lo) * fr
}

/// `FIX(v, 31)` split into the table index (clamped to 127) and the 24 bit fraction.
#[inline]
fn split(v: f32) -> (usize, f32) {
    let v = if v.is_finite() { v.max(0.0) } else { 0.0 };
    let p = (f64::from(v) * 2_147_483_648.0)
        .round_ties_even()
        .min(2_147_483_647.0) as i64;
    (
        (((p >> 24) & 0xFF) as usize).min(IDX_MAX),
        (p & 0xFF_FFFF) as f32 / 16_777_216.0,
    )
}

/// Coefficients of one call (frame words `-24, -22, -20, +10, +12, +14` and `kh`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Stage3Coefs {
    /// `h = 1 - (X1^2 + X1 X2)` (frame `-24`).
    pub h: f32,
    /// `g = 2 U` (frame `-22`).
    pub g: f32,
    /// `V` (frame `-20`).
    pub v: f32,
    /// `kc` (frame `+10`).
    pub kc: f32,
    /// `2 X1` (frame `+12`).
    pub x1x2: f32,
    /// `X2` (frame `+14`).
    pub x2: f32,
    /// `kc h`, the input weight of `n`.
    pub kh: f32,
}

impl Stage3Coefs {
    /// Coefficients for the cutoff coefficient `r` (`tan(pi fc / fs)`, `>= 0`) and the table
    /// position of the resonance word.
    ///
    /// VERIFIED: equal to `tools/synth_filter_t3.py::call_coefficients` within float32 rounding
    /// (test `coefficient_words_match_the_model`, 5 rows).
    #[inline]
    pub fn new(r: f32, pos: TablePos) -> Self {
        let (i, fr) = split(r * 0.5);
        let g = 2.0 * lerp(&T1, i, fr);
        let v = lerp(&T2, i, fr);
        let wv = lerp(&T0, pos.idx.min(IDX_MAX), pos.fr.clamp(0.0, 1.0));
        let (i2, f2) = split(wv * 0.5);
        let x1 = lerp(&T1, i2, f2);
        let x2 = lerp(&T2, i2, f2);
        let kc = (g * K_GAIN + QUARTER).clamp(-CLIP, CLIP);
        let x1x2 = 2.0 * x1;
        let h = 1.0 - (x1x2 * x1x2 * 0.25 + x2 * x1x2 * 0.5);
        Self {
            h,
            g,
            v,
            kc,
            x1x2,
            x2,
            kh: kc * h,
        }
    }
}

/// Flushes values too small to matter to zero so the states never become denormal.
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-24 {
        0.0
    } else {
        x
    }
}

/// State of one lane (frame words `-10, -8, -6, -4, -2`). `Copy`, no heap.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FwStage3 {
    a: f32,
    d: f32,
    b: f32,
    c: f32,
    e: f32,
}

impl FwStage3 {
    /// Filters one sample. A non-finite input is treated as silence; a non-finite result resets.
    #[inline]
    pub fn process(&mut self, x: f32, co: &Stage3Coefs) -> f32 {
        let x = if x.is_finite() { x } else { 0.0 };
        let Self { a, d, b, c, e } = *self;
        let p = b + co.g * a;
        let e_new = e + co.x1x2 * c;
        let m = ((d + x) * 0.5 - p) - co.v * a;
        let n = (x * co.kh - e_new) - co.x2 * c;
        let y = n + (b + p) * 0.5;
        let c_new = (c + co.x1x2 * n).clamp(-CLIP, CLIP);
        let a_new = (co.g * m + a).clamp(-CLIP, CLIP);
        let next = Self {
            a: flush(a_new),
            d: x,
            b: flush(p),
            c: flush(c_new),
            e: flush(e_new),
        };
        if y.is_finite() && next.e.is_finite() && next.b.is_finite() && next.a.is_finite() {
            *self = next;
            y
        } else {
            *self = Self::default();
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(idx: usize) -> TablePos {
        TablePos { idx, fr: 0.0 }
    }

    #[test]
    fn dc_response_is_finite_and_bounded() {
        let co = Stage3Coefs::new(0.3, pos(0));
        let mut f = FwStage3::default();
        let mut y = 0.0;
        for _ in 0..20_000 {
            y = f.process(0.5, &co);
        }
        assert!(y.is_finite() && y.abs() < 4.0, "dc {y}");
    }

    #[test]
    fn split_is_guarded_and_matches_the_word_fields() {
        assert_eq!(split(f32::NAN), (0, 0.0));
        assert_eq!(split(-1.0), (0, 0.0));
        assert_eq!(split(1e9).0, IDX_MAX);
        // 0.25 * 2^31 = 0x2000_0000: index 32, fraction 0
        assert_eq!(split(0.25), (32, 0.0));
        assert_eq!(split(0.2578125), (33, 0.0));
    }

    #[test]
    fn cutoff_zero_gives_the_table_start() {
        let co = Stage3Coefs::new(0.0, pos(0));
        assert_eq!(co.g, 0.0, "T1[0] = 0");
        assert_eq!(co.v, 2.0, "T2[0] = 2");
        assert_eq!(co.kc, 0.25, "kc = 4.5 * 0 + 0.25");
    }

    #[test]
    fn coefficients_grow_with_the_cutoff_and_clip_at_one() {
        let lo = Stage3Coefs::new(0.1, pos(64));
        let hi = Stage3Coefs::new(1.0, pos(64));
        assert!(hi.g > lo.g);
        assert_eq!(hi.kc, 1.0, "4.5 g + 0.25 clips at 1");
        assert!(Stage3Coefs::new(f32::NAN, pos(64)).g.is_finite());
    }

    #[test]
    fn states_stay_bounded_and_non_finite_input_is_silence() {
        let co = Stage3Coefs::new(0.5, TablePos { idx: 127, fr: 0.9 });
        let mut f = FwStage3::default();
        for n in 0..50_000 {
            let x = if n % 977 == 0 { 1.0 } else { 0.0 };
            let y = f.process(x, &co);
            assert!(y.is_finite() && y.abs() < 50.0, "{n}: {y}");
        }
        assert!(f.process(f32::NAN, &co).is_finite());
    }
}
