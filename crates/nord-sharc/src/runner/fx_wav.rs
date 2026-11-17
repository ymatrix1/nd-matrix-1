//! WAV reader for the live FX runs (split out of `fx_run.rs`).
/// Reads a RIFF WAV (PCM 16 bit or float 32 bit, any channel count) into per-channel floats and
/// the sample rate. Local files only; feeds a PNO lane WAV into the FX run.
pub fn read_wav(bytes: &[u8]) -> Result<(Vec<Vec<f32>>, u32), String> {
    let bad = |s: &str| format!("bad WAV: {s}");
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("no RIFF/WAVE header"));
    }
    let (mut pos, mut fmt, mut data) = (12, None, None);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body = bytes
            .get(pos + 8..(pos + 8 + len).min(bytes.len()))
            .unwrap_or(&[]);
        match id {
            b"fmt " if body.len() >= 16 => {
                let u16_at = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
                let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                fmt = Some((u16_at(0), usize::from(u16_at(2)), rate, u16_at(14)));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    let ((tag, ch, rate, bits), data) = fmt.zip(data).ok_or_else(|| bad("missing fmt or data"))?;
    let ch = ch.max(1);
    let step = usize::from(bits / 8);
    if step == 0 {
        return Err(bad("zero sample size"));
    }
    let n = data.len() / (step * ch);
    let mut out = vec![Vec::with_capacity(n); ch];
    for f in 0..n {
        for (c, o) in out.iter_mut().enumerate() {
            let b = &data[(f * ch + c) * step..][..step];
            o.push(match (tag, bits) {
                (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                (1, 16) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
                _ => return Err(bad("only PCM16 and float32 are supported")),
            });
        }
    }
    Ok((out, rate))
}

#[cfg(test)]
mod tests {
    use super::super::audio::wav_bytes;
    use super::*;

    #[test]
    fn wav_reader_accepts_float_and_pcm16_and_rejects_junk() {
        let (ch, rate) = read_wav(&wav_bytes(&[vec![0.5, -0.25], vec![1.0, 0.0]], 48_000)).unwrap();
        assert_eq!((rate, ch), (48_000, vec![vec![0.5, -0.25], vec![1.0, 0.0]]));
        let mut pcm = Vec::new();
        pcm.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        pcm.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 1] {
            pcm.extend_from_slice(&v.to_le_bytes());
        }
        pcm.extend_from_slice(&44_100u32.to_le_bytes());
        pcm.extend_from_slice(&88_200u32.to_le_bytes());
        pcm.extend_from_slice(&2u16.to_le_bytes());
        pcm.extend_from_slice(&16u16.to_le_bytes());
        pcm.extend_from_slice(b"data");
        pcm.extend_from_slice(&4u32.to_le_bytes());
        pcm.extend_from_slice(&16384i16.to_le_bytes());
        pcm.extend_from_slice(&(-32768i16).to_le_bytes());
        assert_eq!(read_wav(&pcm).unwrap(), (vec![vec![0.5, -1.0]], 44_100));
        assert!(read_wav(b"nope").is_err());
        assert!(read_wav(&pcm[..30]).is_err());
    }
}
