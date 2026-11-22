//! Native OS file-picker helpers for `.ns4b` backups, sound files, and Nord OS updates.

use std::path::PathBuf;

/// Allowed extensions for the Backup & Sound Library file picker.
pub const BACKUP_AND_SOUND_EXTS: &[&str] = &["ns4b", "ns4p", "npno", "nsmp4"];

/// Allowed extensions for the Nord Stage 4 OS Update file picker.
pub const OS_UPDATE_EXTS: &[&str] = &["zip", "exe", "dmg", "bin"];

/// Open the native OS File Explorer to pick a `.ns4b` backup, `.ns4p` program, `.npno` piano, or
/// `.nsmp4` sample file.
#[cfg(feature = "app")]
pub fn pick_backup_or_sound() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open Nord Stage 4 Backup or Sound File")
        .add_filter("Nord Stage 4 Files", BACKUP_AND_SOUND_EXTS)
        .add_filter("All Files", &["*"])
        .pick_file()
}

/// Fallback when compiled without the `app` feature (DAW plugin host).
#[cfg(not(feature = "app"))]
pub fn pick_backup_or_sound() -> Option<PathBuf> {
    None
}

/// Open the native OS File Explorer to pick a Nord Stage 4 OS update (`.zip`, `.exe`, `.dmg`, `.bin`).
#[cfg(feature = "app")]
pub fn pick_os_update() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Select Nord Stage 4 OS Update")
        .add_filter("Nord OS Update", OS_UPDATE_EXTS)
        .add_filter("All Files", &["*"])
        .pick_file()
}

/// Fallback when compiled without the `app` feature (DAW plugin host).
#[cfg(not(feature = "app"))]
pub fn pick_os_update() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_extensions_cover_all_supported_nord_formats() {
        assert!(BACKUP_AND_SOUND_EXTS.contains(&"ns4b"));
        assert!(BACKUP_AND_SOUND_EXTS.contains(&"ns4p"));
        assert!(BACKUP_AND_SOUND_EXTS.contains(&"npno"));
        assert!(BACKUP_AND_SOUND_EXTS.contains(&"nsmp4"));
        assert!(OS_UPDATE_EXTS.contains(&"zip"));
        assert!(OS_UPDATE_EXTS.contains(&"exe"));
        assert!(OS_UPDATE_EXTS.contains(&"dmg"));
    }
}
