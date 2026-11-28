//! A whole `.ns4b`-shaped archive from entries: `ZIP_STORED`, real CRCs, the epoch date the
//! writer uses for added entries (`backup_writer.rs`). The test fixtures of the sound library
//! (`library_setting.rs`, the plugin's `auto_library` and `tests/load_path.rs`) build their
//! backups with it, so one header writer serves the rewrite and the fixtures.

use super::backup_writer::{local_entry, new_central_record, u32_offset, DOS_DATE_EPOCH, EOCD_SIG};
use super::CbinError;

/// A stored archive holding `entries` (`(name, data)`), in that order.
pub fn stored_archive(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, CbinError> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = u32_offset(out.len() as u64, "local header offset")?;
        out.extend_from_slice(&local_entry([20, 0], 0, 0, DOS_DATE_EPOCH, name, data)?);
        central.extend_from_slice(&new_central_record(name, data, offset, 0));
    }
    let cd_offset = u32_offset(out.len() as u64, "central directory offset")?;
    let cd_size = u32_offset(central.len() as u64, "central directory size")?;
    let count = u16::try_from(entries.len())
        .ok()
        .filter(|&c| c != u16::MAX)
        .ok_or_else(|| {
            CbinError::BitstreamError("too many entries for a non-ZIP64 archive".into())
        })?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&EOCD_SIG);
    out.extend_from_slice(&[0u8; 4]); // this disk, central directory disk
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbin::Ns4BackupArchive;
    use std::io::Cursor;

    #[test]
    fn the_archive_opens_and_its_entries_read_back_with_matching_crcs() {
        let bytes = stored_archive(&[("Programs/A.ns4p", b"alpha"), ("x/b.bin", b"")]).unwrap();
        let archive = Ns4BackupArchive::from_bytes(&bytes).unwrap();
        let names: Vec<&str> = archive.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Programs/A.ns4p", "x/b.bin"]);
        let mut r = Cursor::new(&bytes);
        assert_eq!(
            archive.read_entry(&mut r, "Programs/A.ns4p").unwrap(),
            b"alpha"
        );
        assert_eq!(archive.read_entry(&mut r, "x/b.bin").unwrap(), b"");
        assert!(stored_archive(&[]).unwrap().len() == 22);
    }
}
