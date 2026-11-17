//! Minimal zip reader: stored (0) and deflate (8) entries, no zip64, no encryption.

use super::MAX_BYTES;
use miniz_oxide::inflate::decompress_to_vec_with_limit;

fn le16(b: &[u8], at: usize) -> Option<usize> {
    let w = b.get(at..at + 2)?;
    Some(u16::from_le_bytes([w[0], w[1]]) as usize)
}

fn le32(b: &[u8], at: usize) -> Option<usize> {
    let w = b.get(at..at + 4)?;
    Some(u32::from_le_bytes([w[0], w[1], w[2], w[3]]) as usize)
}

pub(super) fn is_zip(b: &[u8]) -> bool {
    b.starts_with(b"PK\x03\x04") || b.starts_with(b"PK\x05\x06")
}

/// `(name, bytes)` of every file entry, read through the central directory.
pub(super) fn entries(b: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    entries_within(b, MAX_BYTES)
}

/// [`entries`] with a cap on the total unpacked size of all entries.
fn entries_within(b: &[u8], budget: usize) -> Result<Vec<(String, Vec<u8>)>, String> {
    let bad = || "the zip is damaged".to_string();
    let lo = b.len().saturating_sub(22 + 65_535);
    let eocd = (lo..b.len().saturating_sub(21))
        .rev()
        .find(|&i| b[i..].starts_with(b"PK\x05\x06"))
        .ok_or_else(bad)?;
    let count = le16(b, eocd + 10).ok_or_else(bad)?;
    let mut at = le32(b, eocd + 16).ok_or_else(bad)?;
    let mut out = Vec::new();
    let mut budget = budget;
    for _ in 0..count {
        if le32(b, at) != Some(0x0201_4B50) {
            return Err(bad());
        }
        let method = le16(b, at + 10).ok_or_else(bad)?;
        let csize = le32(b, at + 20).ok_or_else(bad)?;
        let usize_ = le32(b, at + 24).ok_or_else(bad)?;
        let nlen = le16(b, at + 28).ok_or_else(bad)?;
        let xlen = le16(b, at + 30).ok_or_else(bad)?;
        let clen = le16(b, at + 32).ok_or_else(bad)?;
        let local = le32(b, at + 42).ok_or_else(bad)?;
        let raw_name = b.get(at + 46..at + 46 + nlen).ok_or_else(bad)?;
        let name = String::from_utf8_lossy(raw_name).into_owned();
        at += 46 + nlen + xlen + clen;
        if name.ends_with('/') || usize_ > MAX_BYTES {
            continue;
        }
        budget = budget
            .checked_sub(usize_)
            .ok_or("the zip unpacks to more data than expected")?;
        if le32(b, local) != Some(0x0403_4B50) {
            return Err(bad());
        }
        let skip = le16(b, local + 26).ok_or_else(bad)? + le16(b, local + 28).ok_or_else(bad)?;
        let start = local + 30 + skip;
        let raw = b.get(start..start + csize).ok_or_else(bad)?;
        let data = match method {
            0 => raw.to_vec(),
            8 => decompress_to_vec_with_limit(raw, MAX_BYTES)
                .map_err(|e| format!("{name}: cannot unpack ({e:?})"))?,
            m => return Err(format!("{name}: unsupported zip method {m}")),
        };
        if data.len() != usize_ {
            return Err(format!("{name}: wrong unpacked size"));
        }
        out.push((name, data));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::support::fake_zip;
    use super::*;

    #[test]
    fn entries_round_trip_stored_and_deflated() {
        for deflate in [false, true] {
            let z = fake_zip(
                &[("a.txt", b"alpha alpha alpha"), ("dir/b.bin", &[1, 2, 3])],
                deflate,
            );
            assert!(is_zip(&z));
            let e = entries(&z).unwrap();
            assert_eq!(e.len(), 2);
            assert_eq!(e[0], ("a.txt".to_string(), b"alpha alpha alpha".to_vec()));
            assert_eq!(e[1].1, vec![1, 2, 3]);
        }
    }

    #[test]
    fn damaged_zips_and_unknown_methods_are_errors() {
        let z = fake_zip(&[("a", b"xyz")], false);
        assert!(entries(&z[..z.len() - 10]).is_err());
        assert!(entries(b"PK\x03\x04 nonsense").is_err());
        let mut m = fake_zip(&[("a", b"xyz")], false);
        let cd = m.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        m[cd + 10] = 99;
        assert!(entries(&m).unwrap_err().contains("unsupported zip method"));
    }

    #[test]
    fn the_total_unpacked_size_is_capped() {
        let z = fake_zip(&[("a", &[7u8; 600]), ("b", &[8u8; 600])], true);
        assert_eq!(entries_within(&z, 1200).unwrap().len(), 2);
        let e = entries_within(&z, 1000).unwrap_err();
        assert!(e.contains("more data than expected"), "{e}");
    }
}
