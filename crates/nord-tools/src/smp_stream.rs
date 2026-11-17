//! Packed sample stream of one `.nsmp4` stroke, as the SY unpacker reads it (D150).
//!
//! The `\0stk` payload of a stroke is a 68-byte header, zero-word padding, the packed blocks and
//! an end sentinel. The SY image consumes the blocks unchanged except for byte order (big endian
//! words in the file, little endian `u32` in the staging buffer, D112), so the stream file is the
//! payload after the padding with every word byte-swapped. Only derived numbers leave this module
//! (the stream file goes to a local scratch directory, never to git).

use nord_core::cbin::sample::{
    decode_nsmp_stk_payload, parse_nsmp_chunks, Nsmp4Instrument, NsmpChunk,
};
use nord_core::sinc::pitch_law::zone_word;

/// Offset of the first word after the 68-byte `\0stk` header (VERIFIED: `decode_nsmp_stk_payload`).
const HEADER_BYTES: usize = 0x44;

/// Numbers describing the chosen stroke (no sample data).
#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub stroke_id: u32,
    pub root_key: u8,
    pub sample_rate: u16,
    pub channels: u8,
    pub frames: usize,
    pub loop_start: Option<usize>,
    pub loop_end: Option<usize>,
    pub gain_db: f32,
    pub words: usize,
    pub zone_key_low: u8,
    pub zone_key_high: u8,
    pub zone_root: u8,
}

impl StreamInfo {
    /// One-line JSON object (`null` for an absent loop).
    #[must_use]
    pub fn to_json(&self) -> String {
        let opt = |v: Option<usize>| v.map_or("null".to_string(), |n| n.to_string());
        format!(
            "{{\"stroke_id\":{},\"root_key\":{},\"sample_rate\":{},\"channels\":{},\"frames\":{},\
\"loop_start\":{},\"loop_end\":{},\"gain_db\":{:.3},\"words\":{},\"zone_key_low\":{},\
\"zone_key_high\":{},\"zone_root\":{}}}",
            self.stroke_id,
            self.root_key,
            self.sample_rate,
            self.channels,
            self.frames,
            opt(self.loop_start),
            opt(self.loop_end),
            self.gain_db,
            self.words,
            self.zone_key_low,
            self.zone_key_high,
            self.zone_root
        )
    }
}

/// Stroke payload whose first big-endian word is `stroke_id`.
#[must_use]
pub fn find_stroke_payload<'a>(chunks: &[NsmpChunk<'a>], stroke_id: u32) -> Option<&'a [u8]> {
    chunks
        .iter()
        .filter(|c| &c.tag == b"\0stk" && c.payload.len() >= 4)
        .find(|c| {
            u32::from_be_bytes([c.payload[0], c.payload[1], c.payload[2], c.payload[3]])
                == stroke_id
        })
        .map(|c| c.payload)
}

/// Little-endian byte stream of the packed blocks of one `\0stk` payload (padding skipped,
/// end sentinel kept, each big-endian word byte-swapped).
pub fn packed_stream(payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() < HEADER_BYTES + 4 {
        return Err("stroke payload shorter than header".into());
    }
    let mut p = HEADER_BYTES;
    while p + 4 <= payload.len() && payload[p..p + 4] == [0, 0, 0, 0] {
        p += 4;
    }
    let body = &payload[p..];
    if !body.len().is_multiple_of(4) {
        return Err(format!(
            "stream body {} bytes is not word aligned",
            body.len()
        ));
    }
    let mut out = Vec::with_capacity(body.len());
    let (words, _) = body.as_chunks::<4>();
    for w in words {
        out.extend_from_slice(&[w[3], w[2], w[1], w[0]]);
    }
    Ok(out)
}

/// Interleaved little-endian `f32` frames (left, right; mono duplicated) of a decoded stroke.
#[must_use]
pub fn pcm_bytes(left: &[f32], right: Option<&[f32]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(left.len() * 8);
    for (n, l) in left.iter().enumerate() {
        let r = right.map_or(*l, |r| r[n]);
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&r.to_le_bytes());
    }
    out
}

/// One zone of an `.nsmp4` map with the pair block word the ARM would send for it.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneRow {
    pub stroke_id: u32,
    pub key_low: u8,
    pub key_high: u8,
    pub vel_low: u8,
    pub vel_high: u8,
    pub root_key: u8,
    pub rate_hz: u16,
    /// Bits of `pitch_law::zone_word(root_key, rate_hz)` (VERIFIED 8 of 8 for Accordion, D65).
    pub word_bits: u32,
}

impl ZoneRow {
    /// One-line JSON object.
    #[must_use]
    pub fn to_json(&self) -> String {
        format!(
            "{{\"stroke_id\":{},\"key_low\":{},\"key_high\":{},\"vel_low\":{},\"vel_high\":{},\"root_key\":{},\"rate_hz\":{},\"word_bits\":{}}}",
            self.stroke_id,
            self.key_low,
            self.key_high,
            self.vel_low,
            self.vel_high,
            self.root_key,
            self.rate_hz,
            self.word_bits
        )
    }
}

/// Zones of the instrument with the stroke rate of each (zones whose stroke is absent are skipped).
pub fn zone_table(bytes: &[u8]) -> Result<Vec<ZoneRow>, String> {
    let inst = Nsmp4Instrument::parse(bytes).map_err(|e| format!("{e:?}"))?;
    Ok(inst
        .zones
        .iter()
        .filter_map(|z| {
            let st = inst.strokes.iter().find(|s| s.stroke_id == z.stroke_id)?;
            Some(ZoneRow {
                stroke_id: z.stroke_id,
                key_low: z.key_low,
                key_high: z.key_high,
                vel_low: z.vel_low,
                vel_high: z.vel_high,
                root_key: z.root_key,
                rate_hz: st.sample_rate,
                word_bits: zone_word(i32::from(z.root_key), f64::from(st.sample_rate)).to_bits(),
            })
        })
        .collect())
}

/// Packed stream, decoded PCM and facts of the stroke `stroke_id` of the image `bytes`.
pub fn stream_for_stroke(
    bytes: &[u8],
    stroke_id: u32,
) -> Result<(StreamInfo, Vec<u8>, Vec<u8>), String> {
    let zone = zone_table(bytes)?
        .into_iter()
        .find(|z| z.stroke_id == stroke_id)
        .ok_or_else(|| format!("no zone for stroke {stroke_id}"))?;
    stream_of(bytes, &zone)
}

/// Stream and facts for `note` / `velocity` of the `.nsmp4` image `bytes`.
pub fn stream_for_note(
    bytes: &[u8],
    note: u8,
    velocity: u8,
) -> Result<(StreamInfo, Vec<u8>), String> {
    let (info, stream, _) = stream_and_pcm(bytes, note, velocity)?;
    Ok((info, stream))
}

/// Like [`stream_for_note`] plus the decoded PCM (`nord-core` decoder, interleaved `f32`).
pub fn stream_and_pcm(
    bytes: &[u8],
    note: u8,
    velocity: u8,
) -> Result<(StreamInfo, Vec<u8>, Vec<u8>), String> {
    let zone = zone_table(bytes)?
        .into_iter()
        .find(|z| {
            note >= z.key_low
                && note <= z.key_high
                && velocity >= z.vel_low
                && velocity <= z.vel_high
        })
        .ok_or_else(|| format!("no zone for note {note} velocity {velocity}"))?;
    stream_of(bytes, &zone)
}

fn stream_of(bytes: &[u8], zone: &ZoneRow) -> Result<(StreamInfo, Vec<u8>, Vec<u8>), String> {
    let chunks = parse_nsmp_chunks(bytes).map_err(|e| format!("{e:?}"))?;
    let payload = find_stroke_payload(&chunks, zone.stroke_id)
        .ok_or_else(|| format!("no stroke {}", zone.stroke_id))?;
    let stream = packed_stream(payload)?;
    let dec = decode_nsmp_stk_payload(payload).map_err(|e| format!("{e:?}"))?;
    let info = StreamInfo {
        stroke_id: zone.stroke_id,
        root_key: dec.root_key,
        sample_rate: dec.sample_rate,
        channels: dec.channels,
        frames: dec.left.len(),
        loop_start: dec.loop_start,
        loop_end: dec.loop_end,
        gain_db: dec.gain_db,
        words: stream.len() / 4,
        zone_key_low: zone.key_low,
        zone_key_high: zone.key_high,
        zone_root: zone.root_key,
    };
    let pcm = pcm_bytes(&dec.left, dec.right.as_deref());
    Ok((info, stream, pcm))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(id: u32, body: &[[u8; 4]]) -> Vec<u8> {
        let mut p = vec![0u8; HEADER_BYTES];
        p[..4].copy_from_slice(&id.to_be_bytes());
        p.extend_from_slice(&[0; 8]); // zero padding words
        for w in body {
            p.extend_from_slice(w);
        }
        p
    }

    #[test]
    fn packed_stream_skips_padding_and_swaps_words() {
        let p = payload(7, &[[1, 2, 3, 4], [0x00, 0x80, 0x00, 0x20]]);
        assert_eq!(
            packed_stream(&p).unwrap(),
            vec![4, 3, 2, 1, 0x20, 0, 0x80, 0]
        );
    }

    #[test]
    fn packed_stream_rejects_short_and_unaligned() {
        assert!(packed_stream(&[0; 10]).is_err());
        let mut p = payload(1, &[[1, 1, 1, 1]]);
        p.push(9);
        assert!(packed_stream(&p).is_err());
    }

    #[test]
    fn stroke_lookup_matches_first_word_of_stk_chunks_only() {
        let a = payload(5, &[[1, 0, 0, 0]]);
        let b = payload(9, &[[2, 0, 0, 0]]);
        let chunks = [
            NsmpChunk {
                tag: *b"\0stk",
                chunk_id: 1,
                length: a.len() as u32,
                payload: &a,
            },
            NsmpChunk {
                tag: *b"\0map",
                chunk_id: 1,
                length: b.len() as u32,
                payload: &b,
            },
            NsmpChunk {
                tag: *b"\0stk",
                chunk_id: 2,
                length: b.len() as u32,
                payload: &b,
            },
        ];
        assert_eq!(find_stroke_payload(&chunks, 9).unwrap().len(), b.len());
        assert!(find_stroke_payload(&chunks, 4).is_none());
    }

    #[test]
    fn json_marks_missing_loop_as_null() {
        let info = StreamInfo {
            stroke_id: 1,
            root_key: 60,
            sample_rate: 44100,
            channels: 2,
            frames: 10,
            loop_start: None,
            loop_end: Some(8),
            gain_db: 1.5,
            words: 4,
            zone_key_low: 0,
            zone_key_high: 127,
            zone_root: 60,
        };
        let j = info.to_json();
        assert!(j.contains("\"loop_start\":null") && j.contains("\"loop_end\":8"));
        assert!(j.contains("\"gain_db\":1.500"));
    }

    #[test]
    fn pcm_bytes_interleaves_and_duplicates_mono() {
        let st = pcm_bytes(&[1.0, 2.0], Some(&[3.0, 4.0]));
        let f: Vec<f32> = st
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(f, vec![1.0, 3.0, 2.0, 4.0]);
        let mono = pcm_bytes(&[5.0], None);
        assert_eq!(mono.len(), 8);
        assert_eq!(&mono[..4], &mono[4..]);
    }

    #[test]
    fn stream_for_note_rejects_non_nsmp_bytes() {
        assert!(stream_for_note(&[0u8; 100], 60, 100).is_err());
    }

    #[test]
    fn zone_table_and_stroke_lookup_reject_non_nsmp_bytes() {
        assert!(zone_table(b"nope").is_err());
        assert!(stream_for_stroke(b"nope", 1).is_err());
    }

    #[test]
    fn zone_row_json_lists_every_field() {
        let z = ZoneRow {
            stroke_id: 3,
            key_low: 0,
            key_high: 127,
            vel_low: 1,
            vel_high: 127,
            root_key: 60,
            rate_hz: 48000,
            word_bits: 7,
        };
        assert_eq!(
            z.to_json(),
            "{\"stroke_id\":3,\"key_low\":0,\"key_high\":127,\"vel_low\":1,\"vel_high\":127,\"root_key\":60,\"rate_hz\":48000,\"word_bits\":7}"
        );
    }
}
