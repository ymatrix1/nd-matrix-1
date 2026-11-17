//! Decoded PCM refill of the SY source ring (the unpacker bypassed), for sample audio runs (D150).
//!
//! ```text
//! pcm VOICE path=FILE [skip=FRAMES]
//!                     keeps the 128 frame source ring of the voice (`DM(rec + 0x31)`, interleaved
//!                     left / right words, `stride 2`) filled ahead of the resampler read position
//!                     with consecutive stereo frames of FILE (interleaved little-endian float32),
//!                     applied before each `irq`. Source frame `n` sits in ring frame `n mod 128`;
//!                     frames past the end of FILE are written as zeros.
//! ```
//!
//! Provenance:
//! - Ring base `DM(rec + 0x31)` and the 128 frame length: `VERIFIED` ([sy-smp-audio] section 3,
//!   `sy_voice_sample.scn`: prepare state `+0x26 = +0x31`, ring `0xC0000`, 128 frames, `stride 2`).
//! - Read position `DM(rec + 0x2B) >> 25` = ring frame index (prepare state writes `0xFB000000` =
//!   frame 125): `VERIFIED` for the initial value (`sy-smp-audio` s3); that it advances as the
//!   Stage B read index is `EMPIRICAL` (the word advances by the step per frame in the runs).
//! - Source frame 0 at ring frame 0 with the read position starting 3 frames before it
//!   (`ref.pos_frames -2.5` of `sy_voice_sample.scn`): `VERIFIED` (0 error against the engine).
//! - Refilling 96 frames ahead of the read position is `ASSUMED` (any value that keeps the
//!   12 tap kernel inside the window and never overwrites unread frames gives the same output).
//! - That the real unpacker delivers the decoded samples (scaled by `2^-6` and the tail filter,
//!   done host side) is `VERIFIED` only for the first 32 frames ([sy-pitch-entry-writer] s8).
use crate::exec::Runner;
use crate::script::{parse_number, Cmd, REC_BASE, REC_STRIDE};
use crate::signal::Opts;
use std::path::Path;

/// Ring frames of the SY source ring.
const RING_FRAMES: i64 = 128;
/// Frames kept written ahead of the read position (`ASSUMED`, see the module note).
const AHEAD: i64 = 96;
/// Voice record field of the ring base address (`VERIFIED`, prepare state).
const RING_BASE_FIELD: u32 = 0x31;
/// Voice record field of the read position word (index in bits 31..25).
const PHASE_FIELD: u32 = 0x2B;

/// Per-voice state of the `pcm` model.
#[derive(Debug, Clone)]
pub struct PcmRingState {
    pub voice: u32,
    /// Interleaved left / right frames (`2 * frames` values).
    pub samples: Vec<f32>,
    /// Source frames written so far (the next one is `written`).
    pub written: i64,
    /// Unwrapped read position in source frames (starts at `index - 128`).
    pub pos: i64,
    pub last_idx: Option<i64>,
}

impl PcmRingState {
    /// Loads interleaved little-endian float32 frames, skipping `skip` frames.
    pub fn from_file(voice: u32, path: &Path, skip: usize) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("pcm: {}: {e}", path.display()))?;
        if bytes.len() % 8 != 0 {
            return Err(format!(
                "pcm: {}: {} bytes is not a whole number of stereo frames",
                path.display(),
                bytes.len()
            ));
        }
        let samples: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .skip(2 * skip)
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        Ok(Self::new(voice, samples))
    }

    pub fn new(voice: u32, samples: Vec<f32>) -> Self {
        Self {
            voice,
            samples,
            written: 0,
            pos: 0,
            last_idx: None,
        }
    }

    /// Advances the unwrapped read position to ring frame `idx` and returns the source frames
    /// `[written, target)` that must be written now.
    pub fn advance(&mut self, idx: i64) -> std::ops::Range<i64> {
        match self.last_idx {
            None => self.pos = idx - RING_FRAMES,
            Some(last) => self.pos += (idx - last).rem_euclid(RING_FRAMES),
        }
        self.last_idx = Some(idx);
        let from = self.written;
        let target = (self.pos + AHEAD).max(from);
        self.written = target;
        from..target
    }

    /// Left and right value of source frame `n` (zeros past the end of the file).
    pub fn frame(&self, n: i64) -> (f32, f32) {
        let at = usize::try_from(n).ok().map(|n| 2 * n);
        at.and_then(|a| Some((*self.samples.get(a)?, *self.samples.get(a + 1)?)))
            .unwrap_or((0.0, 0.0))
    }
}

/// Parses `pcm VOICE path=FILE [skip=FRAMES]`; `None` when `cmd` is not `"pcm"`.
pub fn parse(cmd: &str, pos: &[String], opts: &Opts) -> Option<Result<Cmd, String>> {
    if cmd != "pcm" {
        return None;
    }
    let voice = match pos.first().and_then(|v| parse_number(v)) {
        Some(v) if v < 56 => v as u32,
        _ => return Some(Err("pcm: needs a voice 0..55".into())),
    };
    let Some(path) = opts.get("path") else {
        return Some(Err("pcm: needs path=FILE".into()));
    };
    let skip = match opts.get("skip") {
        None => 0,
        Some(s) => match parse_number(s) {
            Some(n) => n as usize,
            None => return Some(Err(format!("pcm: bad skip {s:?}"))),
        },
    };
    Some(Ok(Cmd::PcmRing {
        voice,
        path: path.clone(),
        skip,
    }))
}

impl Runner {
    /// Writes the next source frames of every `pcm` voice into its ring before an interrupt.
    pub fn run_pcm_ring(&mut self) -> Result<(), String> {
        let bad = |a: u32| format!("pcm: bad address 0x{a:x}");
        for st in &mut self.pcm_rings {
            let rec = REC_BASE + REC_STRIDE * st.voice;
            let base = self.core.dm_read(rec + RING_BASE_FIELD).map_err(bad)?;
            if base == 0 {
                continue;
            }
            let idx = i64::from(self.core.dm_read(rec + PHASE_FIELD).map_err(bad)? >> 25);
            for n in st.advance(idx) {
                let (l, r) = st.frame(n);
                let slot = base + 2 * (n.rem_euclid(RING_FRAMES) as u32);
                self.core.dm_write(slot, l.to_bits()).map_err(bad)?;
                self.core.dm_write(slot + 1, r.to_bits()).map_err(bad)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(path: &str) -> Opts {
        let mut o = Opts::new();
        o.insert("path".into(), path.into());
        o
    }

    #[test]
    fn parses_pcm_command() {
        let pos = vec!["1".to_string()];
        assert_eq!(
            parse("pcm", &pos, &opts("a.f32")),
            Some(Ok(Cmd::PcmRing {
                voice: 1,
                path: "a.f32".into(),
                skip: 0
            }))
        );
        assert!(parse("stream", &pos, &opts("a")).is_none());
        assert!(parse("pcm", &["56".into()], &opts("a")).unwrap().is_err());
        assert!(parse("pcm", &pos, &Opts::new()).unwrap().is_err());
    }

    #[test]
    fn advance_tracks_wrap_and_writes_ahead_once() {
        let mut st = PcmRingState::new(0, vec![0.0; 1000]);
        // prepare state: read index 125, i.e. source frame -3; fills frames 0..93 first
        assert_eq!(st.advance(125), 0..93);
        assert_eq!(st.pos, -3);
        // two frames later (index 127), then past the wrap (index 1): pos = -3 + 2 + 2 = 1
        assert_eq!(st.advance(127), 93..95);
        assert_eq!(st.advance(1), 95..97);
        assert_eq!(st.pos, 1);
        assert_eq!(st.advance(1), 97..97); // no movement, nothing new
    }

    #[test]
    fn frames_past_the_end_and_before_the_start_are_zero() {
        let st = PcmRingState::new(0, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(st.frame(0), (1.0, 2.0));
        assert_eq!(st.frame(1), (3.0, 4.0));
        assert_eq!(st.frame(2), (0.0, 0.0));
        assert_eq!(st.frame(-1), (0.0, 0.0));
    }

    #[test]
    fn rejects_odd_float_count_files() {
        let dir = std::env::temp_dir().join("sharc_ref_pcm_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("odd.f32");
        std::fs::write(&path, [0u8; 12]).unwrap();
        assert!(PcmRingState::from_file(0, &path, 0).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_writes_ring_words_from_the_record_fields() {
        let mut r = Runner::new(&[0u8; 16], false);
        let samples: Vec<f32> = (0..400).map(|k| k as f32).collect();
        r.pcm_rings.push(PcmRingState::new(0, samples));
        r.run_pcm_ring().unwrap(); // ring base 0: skipped
        assert!(r.pcm_rings[0].last_idx.is_none());
        r.core
            .dm_write(REC_BASE + RING_BASE_FIELD, 0xC0000)
            .unwrap();
        r.core.dm_write(REC_BASE + PHASE_FIELD, 125 << 25).unwrap();
        r.run_pcm_ring().unwrap();
        // frame 10 = samples 20, 21 at ring frame 10
        assert_eq!(f32::from_bits(r.core.dm_read(0xC0000 + 20).unwrap()), 20.0);
        assert_eq!(f32::from_bits(r.core.dm_read(0xC0000 + 21).unwrap()), 21.0);
    }
}
