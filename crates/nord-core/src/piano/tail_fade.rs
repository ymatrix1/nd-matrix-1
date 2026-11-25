//! Fade-out over the last frames of a stroke that has no loop points, so a held note that outlasts
//! its sample ends smoothly instead of stopping abruptly (issue #640, D243).
//!
//! `.npno` strokes loop on the hardware and in the engine (`sample_loop`, D249) and never reach
//! their end. A stroke without loop points (SFZ / WAV banks without `loop_start`, `loop_end`) ends
//! at its last frame, where the voice is freed; library notes fall 40 dB in 3.7 to 5.8 s
//! (`docs/research/piano-decay-attack.md`), so that end is audible as a sudden cut.
//!
//! ASSUMED: fade length 0.25 s and the raised-cosine shape. Chosen to hide the cut without
//! audibly shortening the note; there is no hardware behaviour to copy for such a stroke. A looped
//! stroke needs no fade.

/// Fade length in seconds. ASSUMED (see the module note).
pub const TAIL_FADE_S: f64 = 0.25;

/// Longest share of the stroke the fade may take, so a very short stroke keeps its body.
const MAX_FADE_FRACTION: f64 = 0.25;

/// Gain `[0, 1]` for read position `pos` of a stroke of `len` frames at `rate_hz`: 1 until the
/// last `TAIL_FADE_S` seconds (at most a quarter of the stroke), then a raised-cosine ramp to 0.
#[inline]
pub fn tail_gain(pos: f64, len: usize, rate_hz: f32) -> f32 {
    let len_f = len as f64;
    let fade = (TAIL_FADE_S * f64::from(rate_hz)).min(len_f * MAX_FADE_FRACTION);
    if fade < 1.0 {
        return 1.0;
    }
    let remaining = len_f - pos;
    if remaining >= fade {
        return 1.0;
    }
    if remaining <= 0.0 {
        return 0.0;
    }
    let t = remaining / fade;
    (0.5 - 0.5 * (std::f64::consts::PI * t).cos()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_before_the_fade_zero_at_the_end() {
        let len = 100_000;
        assert_eq!(tail_gain(0.0, len, 35_000.0), 1.0);
        assert_eq!(tail_gain(50_000.0, len, 35_000.0), 1.0);
        assert_eq!(tail_gain(len as f64, len, 35_000.0), 0.0);
        assert_eq!(tail_gain(len as f64 + 5.0, len, 35_000.0), 0.0);
    }

    #[test]
    fn monotonic_and_smooth_over_the_fade() {
        let len = 100_000;
        let fade = (TAIL_FADE_S * 35_000.0) as usize;
        let mut prev = 1.0f32;
        let mut max_step = 0.0f32;
        for i in 0..=fade {
            let g = tail_gain((len - fade + i) as f64, len, 35_000.0);
            assert!(g <= prev + 1e-6, "not monotonic at {i}");
            max_step = max_step.max(prev - g);
            prev = g;
        }
        assert!(max_step < 1e-3, "step too large: {max_step}");
        let mid = tail_gain((len - fade / 2) as f64, len, 35_000.0);
        assert!((mid - 0.5).abs() < 1e-3);
    }

    #[test]
    fn short_stroke_keeps_three_quarters_untouched() {
        let len = 1_000;
        assert_eq!(tail_gain(750.0, len, 35_000.0), 1.0);
        assert!(tail_gain(900.0, len, 35_000.0) < 1.0);
        assert_eq!(tail_gain(0.0, 0, 35_000.0), 1.0);
    }
}
