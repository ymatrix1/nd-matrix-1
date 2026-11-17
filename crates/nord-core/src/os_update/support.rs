//! Synthetic fixtures for the update-file tests: a tiny container wrapped as exe, zip and dmg.

use miniz_oxide::deflate::{compress_to_vec, compress_to_vec_zlib};

/// A 608-byte container: header, image 0 of 0x140 bytes, image 1 of 0x100 bytes.
pub fn container() -> Vec<u8> {
    let mut b = vec![0u8; 0x20 + 0x140 + 0x100];
    for (i, v) in [0x20u32, 0x140, 0x160, 0x100].iter().enumerate() {
        b[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    b[0x30..0x34].copy_from_slice(&1u32.to_be_bytes());
    b[0x34..0x38].copy_from_slice(b"CODE");
    for (i, x) in b.iter_mut().enumerate().skip(0x80) {
        *x = (i % 251) as u8;
    }
    b
}

/// A PE-looking blob: junk, a decoy that is not a container, the container, more junk.
pub fn fake_exe(container: &[u8]) -> Vec<u8> {
    let mut e = b"MZ".to_vec();
    e.resize(300, 0x90);
    e.extend_from_slice(&[0, 0, 0, 0x20, 0, 0, 0, 0x50, 0, 0, 0, 0x70]);
    e.resize(1000, 0xCC);
    e.extend_from_slice(container);
    e.extend_from_slice(&[0xEE; 500]);
    e
}

fn le32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn le16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// A zip with the given entries, stored or deflated.
pub fn fake_zip(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let (mut out, mut cd) = (Vec::new(), Vec::new());
    for (name, data) in entries {
        let body = if deflate {
            compress_to_vec(data, 6)
        } else {
            data.to_vec()
        };
        let method = if deflate { 8u16 } else { 0 };
        let at = out.len() as u32;
        le32(&mut out, 0x0403_4B50);
        out.extend_from_slice(&[20, 0, 0, 0]);
        le16(&mut out, method);
        out.extend_from_slice(&[0; 4]);
        le32(&mut out, 0);
        le32(&mut out, body.len() as u32);
        le32(&mut out, data.len() as u32);
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&body);
        le32(&mut cd, 0x0201_4B50);
        cd.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        le16(&mut cd, method);
        cd.extend_from_slice(&[0; 4]);
        le32(&mut cd, 0);
        le32(&mut cd, body.len() as u32);
        le32(&mut cd, data.len() as u32);
        le16(&mut cd, name.len() as u16);
        cd.extend_from_slice(&[0; 12]);
        le32(&mut cd, at);
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_at = out.len() as u32;
    out.extend_from_slice(&cd);
    le32(&mut out, 0x0605_4B50);
    out.extend_from_slice(&[0; 4]);
    le16(&mut out, entries.len() as u16);
    le16(&mut out, entries.len() as u16);
    le32(&mut out, cd.len() as u32);
    le32(&mut out, cd_at);
    le16(&mut out, 0);
    out
}

pub fn b64(data: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, b)| a | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// One 40-byte UDIF chunk entry.
fn chunk(t: u32, so: u64, sc: u64, co: u64, cl: u64) -> Vec<u8> {
    let mut c = t.to_be_bytes().to_vec();
    c.extend_from_slice(&0u32.to_be_bytes());
    for v in [so, sc, co, cl] {
        c.extend_from_slice(&v.to_be_bytes());
    }
    c
}

/// A UDIF image whose partition starts at sector 7 and holds 3 zero sectors, then `container`
/// cut over a zlib chunk and a raw chunk (so it spans a chunk boundary).
pub fn fake_dmg(container: &[u8]) -> Vec<u8> {
    let mut disk = vec![0u8; 3 * 512];
    disk.extend_from_slice(container);
    disk.resize(disk.len().div_ceil(512) * 512, 0);
    let sectors = disk.len() / 512;
    let split = 5.min(sectors - 1);
    let z = compress_to_vec_zlib(&disk[3 * 512..split * 512], 6);
    let mut data = z.clone();
    let raw_at = data.len();
    data.extend_from_slice(&disk[split * 512..]);
    let mut mish = b"mish".to_vec();
    mish.resize(204, 0);
    mish[8..16].copy_from_slice(&7u64.to_be_bytes());
    mish[16..24].copy_from_slice(&(sectors as u64).to_be_bytes());
    mish[200..204].copy_from_slice(&4u32.to_be_bytes());
    mish.extend(chunk(0, 0, 3, 0, 0));
    mish.extend(chunk(0x8000_0005, 3, (split - 3) as u64, 0, z.len() as u64));
    mish.extend(chunk(
        1,
        split as u64,
        (sectors - split) as u64,
        raw_at as u64,
        (disk.len() - split * 512) as u64,
    ));
    mish.extend(chunk(0xFFFF_FFFF, sectors as u64, 0, 0, 0));
    let plist = format!(
        "<plist><dict><key>blkx</key><array><dict><key>Data</key>\n<data>\n{}\n</data>\n<key>Name</key><string>disk image (Apple_HFS : 4)</string></dict></array></dict></plist>",
        b64(&mish)
    );
    let mut out = data;
    let xml_at = out.len() as u64;
    out.extend_from_slice(plist.as_bytes());
    let mut koly = b"koly".to_vec();
    koly.resize(512, 0);
    koly[0xD8..0xE0].copy_from_slice(&xml_at.to_be_bytes());
    koly[0xE0..0xE8].copy_from_slice(&(plist.len() as u64).to_be_bytes());
    out.extend_from_slice(&koly);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_container_is_608_bytes_and_wrappers_are_larger() {
        assert_eq!(container().len(), 608);
        assert!(fake_exe(&container()).len() > 608);
        assert!(fake_zip(&[("a", b"b")], false).len() > 20);
        assert!(fake_dmg(&container()).len() > 608);
        assert_eq!(b64(b"Man"), "TWFu");
        assert_eq!(b64(b"Ma"), "TWE=");
    }
}
