//! Process-wide holder of the firmware fonts. Empty until an update file loads; callers fall back
//! to their own text painter while [`installed`] is false.

use super::{font_table, parse_font, Font, BAR_FONT};
use crate::render_opts_wave::WAVE_TABLES_ENV;
use std::path::Path;
use std::sync::RwLock;

static FONTS: RwLock<Vec<Font>> = RwLock::new(Vec::new());

/// Whether firmware fonts are loaded.
pub fn installed() -> bool {
    FONTS.read().map(|f| !f.is_empty()).unwrap_or(false)
}

/// A copy of font `index` of the UI font table (0 = 13 px, 3 = 8 px), `None` without fonts.
pub fn font(index: usize) -> Option<Font> {
    FONTS.read().ok()?.get(index).cloned()
}

/// Reads the fonts from the Nord OS update file (or raw `bank0_CODE`) at `path` and installs them.
/// On an error nothing changes and the reason is returned.
pub fn install_from_path(path: &Path) -> Result<usize, String> {
    // An update file (`.exe`, `.zip`, `.dmg`, container), else a raw `bank0_CODE` image.
    let code = match crate::dsp_image::bank0_code(path) {
        Ok(code) => code,
        Err(why) => crate::os_update::read_capped(path)
            .map_err(|_| why.clone())
            .and_then(|raw| font_table(&raw).map(|_| raw).map_err(|_| why))?,
    };
    let mut fonts = font_table(&code).map_err(|e| format!("{}: {e}", path.display()))?;
    // The drawbar bar font follows the 17 UI fonts (index `BAR_FONT_INDEX`); optional.
    if let Ok(bars) = parse_font(&code, BAR_FONT) {
        fonts.push(bars);
    }
    let n = fonts.len();
    if let Ok(mut slot) = FONTS.write() {
        *slot = fonts;
    }
    Ok(n)
}

/// [`install_from_path`] for the file in `NS4_WAVE_TABLES` (the start-up sets it from the saved
/// setting). `Ok(0)` when the variable is unset: the fallback painter stays in use.
pub fn install_from_env() -> Result<usize, String> {
    match std::env::var(WAVE_TABLES_ENV) {
        Ok(p) if !p.trim().is_empty() => install_from_path(Path::new(p.trim())),
        _ => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raw_code_image_installs_and_a_bad_path_changes_nothing() {
        let dir = std::env::temp_dir().join(format!("ns4-oled-font-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(install_from_path(&dir.join("missing.bin")).is_err());
        let junk = dir.join("junk.bin");
        std::fs::write(&junk, [0u8; 64]).unwrap();
        assert!(install_from_path(&junk).is_err());
        let raw = dir.join("code.bin");
        std::fs::write(&raw, super::super::tests::synthetic_code()).unwrap();
        assert_eq!(
            install_from_path(&raw),
            Ok(17),
            "no bar font in the fixture"
        );
        assert!(installed());
        assert_eq!(font(3).map(|f| f.height), Some(11));
        assert!(font(super::super::BAR_FONT_INDEX).is_none());
        assert!(install_from_path(&junk).is_err());
        assert!(installed(), "a failed install keeps the fonts");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unset_variable_is_not_an_error() {
        if std::env::var(WAVE_TABLES_ENV).is_err() {
            assert_eq!(install_from_env(), Ok(0));
        }
    }
}
