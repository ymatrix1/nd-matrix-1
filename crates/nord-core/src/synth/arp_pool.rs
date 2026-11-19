//! Note pool of the firmware arpeggiator walk (`SynthLayer::arp_firmware_walk`, D166).
//!
//! The ARM walks UP, DOWN and UP/DOWN over the held chord stacked in octaves: every chord note
//! `n + 12 k` (`k >= 0`) whose distance from the lowest held note is below the span of the RANGE
//! value. UP/DOWN mirrors over that pool without repeating the end points.
//!
//! Span rule, fitted to ARM replays of A:16 Horizon Whl layers C and A (`docs/research/arp-fw-vs-native.md`
//! s7, `docs/research/data/arp-fw-range.json`): `span = firmware_span(raw)` semitones, strict `<`,
//! `raw` = stored RANGE value `0..=127`. A raw value of 0 gives one octave (triad 60 64 67, single
//! key 60), 47 gives the top root (60 64 67 72 76 79 84), 127 gives four octaves for a single key
//! (60 72 84 96).

/// Largest pool the audio thread builds (a held chord of up to 64 notes over four octaves).
pub(super) const POOL_MAX: usize = 256;

/// `(raw, span)` knots of the RANGE law, linear in between, flat after the last.
///
/// EMPIRICAL: each knot lies inside the bracket that 14 ARM captures (layers C and A, 30 key sets,
/// raw 0, 7, 14, 16, 28, 32, 42, 47, 57, 63, 85, 95, 126, 127) leave for the span of that raw value:
/// the pool holds the chord notes whose offset from the lowest key is below the span. A straight
/// line `12 + raw / 3` fits the 30 brackets except raw 7, 14 and 57, so the law is piecewise: flat
/// 12 up to raw 7, then steeper. Brackets (semitones): raw 0 and 7 (7, 12], 14 (12, 15], 16 (16, 19],
/// 28 and 32 (19, 24], 42 (24, 27], 47 (24, 28], 57 and 63 (31, 36], 95 and 126 (43, 48]; the knots
/// are bracket edges or interior points, so the error against the real law is at most the bracket
/// width (1 to 5 semitones, wider for raw 85). Raw 1..6, 8..13 and 64..94 are interpolated (ASSUMED),
/// and the hardware step of 4 semitones per detent was not seen.
const SPAN_KNOTS: [(f32, f32); 12] = [
    (0.0, 12.0),
    (7.0, 12.0),
    (14.0, 15.0),
    (16.0, 16.5),
    (32.0, 24.0),
    (42.0, 26.5),
    (47.0, 28.0),
    (57.0, 31.5),
    (63.0, 35.5),
    (95.0, 44.0),
    (126.0, 48.0),
    (127.0, 48.0),
];

/// Span in semitones (exclusive) of the stored RANGE value `raw` (`0..=127`).
pub(super) fn firmware_span(raw: f32) -> f32 {
    let raw = raw.clamp(0.0, 127.0);
    for w in SPAN_KNOTS.windows(2) {
        let ((r0, s0), (r1, s1)) = (w[0], w[1]);
        if raw <= r1 {
            return s0 + (s1 - s0) * (raw - r0) / (r1 - r0);
        }
    }
    SPAN_KNOTS[SPAN_KNOTS.len() - 1].1
}

/// One pool note: the sounding note and the index of the held key it derives from.
#[derive(Clone, Copy, Default)]
pub(super) struct PoolNote {
    pub note: u8,
    pub source: u8,
}

/// Fills `out` with the pool of the ascending held notes `held` and returns its length. Octave 0
/// always holds every held note; higher octaves hold the notes below `span` above the lowest key.
/// ASSUMED: a chord wider than the span still plays whole in octave 0, and a note that appears
/// twice (a held octave pair) stays twice.
pub(super) fn firmware_pool(held: &[u8], span: f32, out: &mut [PoolNote; POOL_MAX]) -> usize {
    let Some(&lowest) = held.first() else {
        return 0;
    };
    let mut len = 0;
    for k in 0..5u16 {
        for &n in held {
            let value = u16::from(n) + 12 * k;
            if value > 127 || len == POOL_MAX {
                continue;
            }
            if k > 0 && f32::from(value - u16::from(lowest)) >= span {
                continue;
            }
            out[len] = PoolNote {
                note: value as u8,
                source: n,
            };
            len += 1;
        }
    }
    // Chords wider than an octave interleave the octaves: order by pitch (stable, insertion sort
    // on an almost sorted array, no allocation).
    for i in 1..len {
        let x = out[i];
        let mut j = i;
        while j > 0 && out[j - 1].note > x.note {
            out[j] = out[j - 1];
            j -= 1;
        }
        out[j] = x;
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(held: &[u8], raw: f32) -> Vec<u8> {
        let mut buf = [PoolNote::default(); POOL_MAX];
        let n = firmware_pool(held, firmware_span(raw), &mut buf);
        buf[..n].iter().map(|p| p.note).collect()
    }

    #[test]
    fn span_is_monotonic_flat_at_both_ends_and_capped_at_four_octaves() {
        assert_eq!(firmware_span(0.0), 12.0);
        assert_eq!(firmware_span(7.0), 12.0);
        assert_eq!(firmware_span(127.0), 48.0);
        assert_eq!(firmware_span(500.0), 48.0);
        assert_eq!(firmware_span(-3.0), 12.0);
        let mut last = 0.0;
        for r in 0..=127 {
            let s = firmware_span(r as f32);
            assert!(s >= last, "monotonic at {r}");
            last = s;
        }
    }

    /// `(raw, lowest excluded offset, highest included offset)` of the ARM captures: the span of
    /// `raw` must satisfy `included < span <= excluded` (`docs/research/data/arp-fw-range.json`).
    const BRACKETS: [(f32, f32, f32); 13] = [
        (0.0, 12.0, 7.0),
        (7.0, 12.0, 7.0),
        (14.0, 15.0, 12.0),
        (16.0, 19.0, 16.0),
        (28.0, 24.0, 19.0),
        (32.0, 24.0, 19.0),
        (42.0, 27.0, 24.0),
        (47.0, 28.0, 24.0),
        (57.0, 36.0, 31.0),
        (63.0, 36.0, 31.0),
        (95.0, 48.0, 43.0),
        (126.0, 48.0, 43.0),
        (127.0, 48.0, 36.0),
    ];

    #[test]
    fn span_sits_inside_every_arm_bracket() {
        for (raw, excluded, included) in BRACKETS {
            let s = firmware_span(raw);
            assert!(
                included < s && s <= excluded,
                "raw {raw}: span {s} not in ({included}, {excluded}]"
            );
        }
    }

    #[test]
    fn pools_match_the_arm_captures_of_layer_c() {
        // arp-fw-range-*.json, keys 60 64 67 (Up cycles) and key 60 (single).
        assert_eq!(pool(&[60, 64, 67], 0.0), [60, 64, 67]);
        assert_eq!(pool(&[60, 64, 67], 7.0), [60, 64, 67]);
        assert_eq!(pool(&[60, 64, 67], 16.0), [60, 64, 67, 72, 76]);
        assert_eq!(pool(&[60, 64, 67], 28.0), [60, 64, 67, 72, 76, 79]);
        assert_eq!(pool(&[60, 64, 67], 32.0), [60, 64, 67, 72, 76, 79]);
        assert_eq!(pool(&[60, 64, 67], 47.0), [60, 64, 67, 72, 76, 79, 84]);
        assert_eq!(
            pool(&[60, 64, 67], 57.0),
            [60, 64, 67, 72, 76, 79, 84, 88, 91]
        );
        assert_eq!(pool(&[60], 0.0), [60]);
        assert_eq!(pool(&[60], 16.0), [60, 72]);
        assert_eq!(pool(&[60], 47.0), [60, 72, 84]);
        assert_eq!(pool(&[60], 85.0), [60, 72, 84, 96]);
        assert_eq!(pool(&[60], 127.0), [60, 72, 84, 96]);
        assert_eq!(
            pool(&[60, 64, 67], 85.0)[..11],
            [60, 64, 67, 72, 76, 79, 84, 88, 91, 96, 100]
        );
    }

    #[test]
    fn pools_match_the_arm_captures_of_layer_a() {
        // keys 40 43 47 (Up cycles, A:16 layer A).
        assert_eq!(pool(&[40, 43, 47], 0.0), [40, 43, 47]);
        assert_eq!(pool(&[40, 43, 47], 14.0), [40, 43, 47, 52]);
        assert_eq!(pool(&[40, 43, 47], 42.0), [40, 43, 47, 52, 55, 59, 64]);
        assert_eq!(
            pool(&[40, 43, 47], 63.0),
            [40, 43, 47, 52, 55, 59, 64, 67, 71]
        );
        assert_eq!(pool(&[40, 43, 47], 126.0).len(), 12);
    }

    #[test]
    fn pool_keeps_the_source_key_and_orders_a_wide_chord_by_pitch() {
        let mut buf = [PoolNote::default(); POOL_MAX];
        let n = firmware_pool(&[60, 76], firmware_span(47.0), &mut buf);
        let notes: Vec<u8> = buf[..n].iter().map(|p| p.note).collect();
        assert_eq!(notes, [60, 72, 76, 84]);
        assert_eq!(buf[1].source, 60);
        assert_eq!(buf[2].source, 76);
    }

    #[test]
    fn pool_stops_at_note_127_and_handles_no_keys() {
        assert_eq!(pool(&[], 0.0), Vec::<u8>::new());
        assert!(pool(&[120], 127.0).iter().all(|&n| n <= 127));
    }
}
