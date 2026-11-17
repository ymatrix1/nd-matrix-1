//! Apple UDIF disk image (`.dmg`) reader: `koly` trailer, XML plist of `mish` block tables, and the
//! zlib / raw / zero chunk types. Decodes the sectors into one flat image; no filesystem is read.
//! ADC and bzip2 chunks are rejected with a clear message (the real v1.66 image uses zlib only).
//!
//! Layout (big endian). `koly` (last 512 B): data fork offset @0x18, plist offset @0xD8, plist
//! length @0xE0. `mish`: first sector @8, sector count @16, data offset @24, chunk count @200,
//! then 40-byte chunks `type, comment, sector, sectors, compressed offset, compressed length`
//! (sector relative to the table's first sector, offset relative to data fork + data offset).

use super::MAX_BYTES;
use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;

const KOLY: usize = 512;
const SECTOR: usize = 512;

fn be64(b: &[u8], at: usize) -> Option<usize> {
    let w: [u8; 8] = b.get(at..at + 8)?.try_into().ok()?;
    usize::try_from(u64::from_be_bytes(w)).ok()
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let w: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(w))
}

pub(super) fn is_dmg(b: &[u8]) -> bool {
    b.len() > KOLY && b[b.len() - KOLY..].starts_with(b"koly")
}

fn unb64(s: &str) -> Vec<u8> {
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// The `mish` tables: every `<key>Data</key><data>..</data>` of the plist, decoded.
fn tables(xml: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(k) = rest.find("<key>Data</key>") {
        rest = &rest[k..];
        let (Some(a), Some(z)) = (rest.find("<data>"), rest.find("</data>")) else {
            break;
        };
        if a < z {
            out.push(unb64(&rest[a + 6..z]));
        }
        rest = &rest[z + 7..];
    }
    out
}

/// The disk image: every partition's sectors laid out at their sector numbers.
pub(super) fn decode(b: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "the disk image is damaged".to_string();
    let k = b.len() - KOLY;
    let fork = be64(b, k + 0x18).ok_or_else(bad)?;
    let xo = be64(b, k + 0xD8).ok_or_else(bad)?;
    let xl = be64(b, k + 0xE0).ok_or_else(bad)?;
    let xend = xo.checked_add(xl).ok_or_else(bad)?;
    let xml = String::from_utf8_lossy(b.get(xo..xend).ok_or_else(bad)?).into_owned();
    let mut image: Vec<u8> = Vec::new();
    for m in tables(&xml).iter().filter(|m| m.starts_with(b"mish")) {
        let first = be64(m, 8).ok_or_else(bad)?;
        let data_off = be64(m, 24).ok_or_else(bad)?;
        let n = be32(m, 200).ok_or_else(bad)? as usize;
        for i in 0..n {
            let e = 204 + 40 * i;
            let t = be32(m, e).ok_or_else(bad)?;
            if t == 0xFFFF_FFFF {
                break;
            }
            let so = be64(m, e + 8).ok_or_else(bad)?;
            let sc = be64(m, e + 16).ok_or_else(bad)?;
            let co = be64(m, e + 24).ok_or_else(bad)?;
            let cl = be64(m, e + 32).ok_or_else(bad)?;
            let at = first
                .checked_add(so)
                .and_then(|s| s.checked_mul(SECTOR))
                .ok_or_else(bad)?;
            let len = sc.checked_mul(SECTOR).ok_or_else(bad)?;
            if at.checked_add(len).ok_or_else(bad)? > MAX_BYTES {
                return Err("the disk image is larger than expected".into());
            }
            let src = || {
                let s = fork.checked_add(data_off)?.checked_add(co)?;
                b.get(s..s.checked_add(cl)?)
            };
            let chunk = match t {
                0 | 2 | 0x7FFF_FFFE => continue,
                1 if cl > len => return Err(bad()),
                1 => src().ok_or_else(bad)?.to_vec(),
                0x8000_0005 => decompress_to_vec_zlib_with_limit(src().ok_or_else(bad)?, len)
                    .map_err(|e| format!("cannot unpack a disk image block ({e:?})"))?,
                0x8000_0006 | 0x8000_0007 => {
                    return Err(
                        "this disk image uses ADC/bzip2 compression, which is not supported".into(),
                    )
                }
                other => return Err(format!("unknown disk image block type {other:#x}")),
            };
            if image.len() < at + chunk.len() {
                image.resize(at + chunk.len(), 0);
            }
            image[at..at + chunk.len()].copy_from_slice(&chunk);
        }
    }
    if image.is_empty() {
        return Err("the disk image holds no data".into());
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::super::support::{container, fake_dmg};
    use super::*;

    #[test]
    fn decode_lays_chunks_at_their_sectors() {
        let d = fake_dmg(&container());
        assert!(is_dmg(&d));
        let img = decode(&d).unwrap();
        let base = (7 + 3) * 512; // partition starts at sector 7, three zero sectors first
        assert_eq!(&img[base..base + 608], &container()[..]);
        assert!(img[7 * 512..base].iter().all(|&x| x == 0));
    }

    #[test]
    fn base64_and_plist_tables_decode() {
        assert_eq!(unb64("TWFu\nTWE="), b"ManMa");
        let t = tables("<key>Data</key>\n<data>\nTWFu\n</data><key>Data</key><data>TWE=</data>");
        assert_eq!(t, vec![b"Man".to_vec(), b"Ma".to_vec()]);
    }

    #[test]
    fn unsupported_compression_and_damage_are_errors() {
        let mut d = fake_dmg(&container());
        assert!(!is_dmg(b"plain"));
        let n = d.len();
        d[n - 512 + 0xD8..n - 512 + 0xE0].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(decode(&d).is_err());
    }

    #[test]
    fn a_raw_chunk_longer_than_its_sectors_is_damage() {
        use super::super::support::b64;
        let mut d = fake_dmg(&container());
        let xml = String::from_utf8_lossy(&d).into_owned();
        let mut mish = tables(&xml).remove(0);
        let old = b64(&mish);
        // third chunk (raw): claim zero sectors while its stored length stays one sector
        mish[204 + 80 + 16..204 + 80 + 24].copy_from_slice(&0u64.to_be_bytes());
        let at = d
            .windows(old.len())
            .position(|w| w == old.as_bytes())
            .unwrap();
        d[at..at + old.len()].copy_from_slice(b64(&mish).as_bytes());
        assert!(decode(&d).unwrap_err().contains("damaged"));
    }
}
