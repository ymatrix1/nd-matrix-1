//! Reads the firmware container out of the official Nord OS update file at run time (D129).
//!
//! The user downloads the update from nordkeyboards.com and hands that one file to the app; nothing
//! extracted is ever written to disk or committed (D22, AGENTS.md rule 6). Accepted inputs:
//!
//! | Input | How the container is found |
//! |---|---|
//! | the container itself | header check at offset 0 |
//! | `Update Nord Stage 4 OS vX.YY.exe` (wxWidgets updater, PE) | header scan; the container is RCDATA 101, stored uncompressed |
//! | `Update Nord Stage 4 OS vX.YY.zip` | stored/deflate entry, then the exe/dmg rules |
//! | `Update Nord Stage 4 OS vX.YY.dmg` | UDIF zlib chunks decoded, then a header scan of the HFS volume |
//!
//! Container header (big endian, `docs/research/ns4-firmware-teardown.md`): `0x20`, image 0 size,
//! image 1 start (`0x20 + image 0 size`), image 1 size; the container is `0x20 + size0 + size1`
//! bytes. VERIFIED (2026-11-17, real v1.66 files, numbers in `docs/research/os-update-input.md`):
//! the container read from the `.exe` (RCDATA 101 at file offset `0x79CB30`) and from the `.dmg`
//! (offset `0x1CC1000` of the decoded image) is byte-identical to the unpacked `fw_v1.66.bin`,
//! 4,037,080 B. The USB driver installer (RCDATA 105) has no such header, so the scan skips it.

mod dmg;
mod zip;

use crate::firmware::FirmwareContainer;

/// Largest input or decoded image accepted (the real files are 12.8 MB and 40 MB decoded).
pub(crate) const MAX_BYTES: usize = 256 << 20;
/// Largest file read from disk (the real update files are 12.8 MB; a `.ns4b` is 3 GB).
pub const MAX_INPUT: u64 = 512 << 20;
const HEADER: usize = 0x20;
/// Nesting limit: zip -> exe/dmg.
const MAX_DEPTH: u32 = 2;

const NOT_AN_UPDATE: &str =
    "this is not a Nord OS update: expected 'Update Nord Stage 4 OS vX.YY' as .exe, .zip or .dmg";

fn be32(b: &[u8], at: usize) -> Option<usize> {
    let w = b.get(at..at + 4)?;
    Some(u32::from_be_bytes([w[0], w[1], w[2], w[3]]) as usize)
}

/// Length of a well-formed container starting exactly at `b[0]`, or `None`.
fn container_len_at(b: &[u8]) -> Option<usize> {
    if be32(b, 0)? != HEADER {
        return None;
    }
    let (i0, i1_at, i1) = (be32(b, 4)?, be32(b, 8)?, be32(b, 12)?);
    if i0 < 0x40 || i1 == 0 || i1_at != HEADER + i0 {
        return None;
    }
    let total = HEADER.checked_add(i0)?.checked_add(i1)?;
    let body = b.get(..total)?;
    let fw = FirmwareContainer::parse(body).ok()?;
    (!fw.image0_segments.is_empty()).then_some(total)
}

/// Finds the container anywhere in `b` (PE resource data, decoded disk image).
pub(crate) fn find_container(b: &[u8]) -> Option<&[u8]> {
    let mut from = 0;
    while let Some(p) = b[from..].windows(4).position(|w| w == [0, 0, 0, 0x20]) {
        let at = from + p;
        if let Some(n) = container_len_at(&b[at..]) {
            return Some(&b[at..at + n]);
        }
        from = at + 1;
    }
    None
}

/// Reads `path` whole, refusing a file larger than [`MAX_INPUT`] before allocating it.
pub fn read_capped(path: &std::path::Path) -> Result<Vec<u8>, String> {
    read_capped_to(path, MAX_INPUT)
}

fn read_capped_to(path: &std::path::Path, cap: u64) -> Result<Vec<u8>, String> {
    let shown = path.display();
    let len = std::fs::metadata(path)
        .map_err(|e| format!("{shown}: {e}"))?
        .len();
    if len > cap {
        return Err(format!(
            "{shown}: file is {} MB, too big to be a Nord OS update (limit {} MB)",
            len >> 20,
            cap >> 20
        ));
    }
    std::fs::read(path).map_err(|e| format!("{shown}: {e}"))
}

fn extract(bytes: &[u8], depth: u32) -> Result<Vec<u8>, String> {
    if let Some(n) = container_len_at(bytes) {
        return Ok(bytes[..n].to_vec());
    }
    if zip::is_zip(bytes) {
        if depth >= MAX_DEPTH {
            return Err("zip nested too deeply".into());
        }
        let mut last = "the zip holds no Nord OS update".to_string();
        for (name, data) in zip::entries(bytes)? {
            match extract(&data, depth + 1) {
                Ok(c) => return Ok(c),
                Err(e) => last = format!("{name}: {e}"),
            }
        }
        return Err(last);
    }
    if dmg::is_dmg(bytes) {
        let image = dmg::decode(bytes)?;
        return find_container(&image)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| NOT_AN_UPDATE.to_string());
    }
    if bytes.starts_with(b"MZ") {
        return find_container(bytes)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "this program holds no Nord Stage 4 firmware (wrong updater?)".into());
    }
    Err(NOT_AN_UPDATE.to_string())
}

/// The firmware container inside `bytes`: the container itself, the official updater `.exe` or
/// `.dmg`, or a `.zip` holding one of them.
pub fn extract_firmware_container(bytes: &[u8]) -> Result<Vec<u8>, String> {
    extract(bytes, 0)
}

#[cfg(test)]
mod support;

#[cfg(test)]
mod tests {
    use super::support::*;
    use super::*;

    #[test]
    fn the_bare_container_passes_through_trimmed() {
        let c = container();
        let mut padded = c.clone();
        padded.extend_from_slice(&[7; 40]);
        assert_eq!(extract_firmware_container(&padded).unwrap(), c);
    }

    #[test]
    fn an_exe_yields_the_container_and_skips_decoys() {
        let exe = fake_exe(&container());
        assert_eq!(extract_firmware_container(&exe).unwrap(), container());
    }

    #[test]
    fn an_exe_without_firmware_is_rejected_plainly() {
        let mut exe = b"MZ".to_vec();
        exe.extend_from_slice(&[0, 0, 0, 0x20, 0, 0, 1, 0, 0, 0, 1, 0x20, 0, 0, 0, 9]);
        exe.resize(4096, 0);
        let e = extract_firmware_container(&exe).unwrap_err();
        assert!(e.contains("no Nord Stage 4 firmware"), "{e}");
    }

    #[test]
    fn a_zip_with_a_stored_or_deflated_exe_works() {
        let exe = fake_exe(&container());
        for deflate in [false, true] {
            let z = fake_zip(&[("readme.txt", b"hi"), ("Update.exe", &exe)], deflate);
            assert_eq!(extract_firmware_container(&z).unwrap(), container());
        }
    }

    #[test]
    fn a_dmg_yields_the_container() {
        let d = fake_dmg(&container());
        assert_eq!(extract_firmware_container(&d).unwrap(), container());
    }

    #[test]
    fn a_zip_wrapping_a_dmg_works() {
        let z = fake_zip(&[("Update.dmg", &fake_dmg(&container()))], true);
        assert_eq!(extract_firmware_container(&z).unwrap(), container());
    }

    #[test]
    fn anything_else_says_it_is_not_a_nord_os_update() {
        for junk in [&b""[..], b"hello world", &[0u8; 5000]] {
            let e = extract_firmware_container(junk).unwrap_err();
            assert!(e.contains("not a Nord OS update"), "{e}");
        }
        let z = fake_zip(&[("a.txt", b"no firmware here")], false);
        assert!(extract_firmware_container(&z).is_err());
    }

    #[test]
    fn a_file_over_the_cap_is_refused_before_it_is_read() {
        let p = std::env::temp_dir().join(format!("ns4-cap-{}.bin", std::process::id()));
        std::fs::write(&p, vec![0u8; 3 << 20]).unwrap();
        assert_eq!(read_capped_to(&p, 4 << 20).unwrap().len(), 3 << 20);
        let e = read_capped_to(&p, 2 << 20).unwrap_err();
        assert!(e.contains("too big") && e.contains("3 MB"), "{e}");
        assert!(read_capped(&p).is_ok());
        assert!(read_capped(&p.with_extension("missing")).is_err());
    }

    #[test]
    fn a_truncated_container_is_not_accepted() {
        let c = container();
        assert!(extract_firmware_container(&c[..c.len() - 1]).is_err());
    }
}
