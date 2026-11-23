//! User-visible product naming (single source for brand-like strings).
//!
//! Rule (docs/specs/visual-style-and-panel.md, "Naming and trademark rule"): our product is
//! "ND Matrix 1". "Nord" and "Nord Stage 4" appear only descriptively ("files for the Nord
//! Stage 4"), never as the name of a window, panel, library or app.
//!
//! ASSUMED: wording below is a project naming choice (D16), not legal advice.

/// Our product name, used for the window class, default program title and chassis wordmark.
pub const PRODUCT_NAME: &str = "ND Matrix 1";

/// Program title shown before any program is loaded.
pub const DEFAULT_PROGRAM_TITLE: &str = PRODUCT_NAME;

/// Lower-case wordmark drawn on the top left of the chassis.
pub const CHASSIS_WORDMARK: &str = "nd matrix 1";

/// Heading of the backup and sample library drawer.
pub const LIBRARY_TITLE: &str = "Backup & Sample Library (Nord Stage 4 files)";

/// Heading of the inline backup browser card in the setup dialog.
pub const BROWSER_CARD_TITLE: &str = "Backup & Sample Browser for Nord Stage 4 (.ns4b)";

/// File-dialog filter label for backups and sound files.
pub const BACKUP_SOUND_FILTER_LABEL: &str = "Backup and sound files for Nord Stage 4";

/// Description of the computer-keyboard card (descriptive, no brand).
pub const COMPUTER_KB_CARD_DESC: &str = "Play the instrument directly from your computer keyboard.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_name_is_nd_matrix_1() {
        assert_eq!(PRODUCT_NAME, "ND Matrix 1");
        assert_eq!(DEFAULT_PROGRAM_TITLE, "ND Matrix 1");
        assert_eq!(CHASSIS_WORDMARK, "nd matrix 1");
    }

    #[test]
    fn nord_appears_only_descriptively() {
        for s in [LIBRARY_TITLE, BROWSER_CARD_TITLE, BACKUP_SOUND_FILTER_LABEL] {
            assert!(!s.starts_with("Nord"), "{s}");
            assert!(
                s.contains("for Nord Stage 4") || s.contains("(Nord Stage 4 files)"),
                "{s}"
            );
        }
        assert!(!COMPUTER_KB_CARD_DESC.contains("Nord"));
        assert!(!PRODUCT_NAME.to_lowercase().contains("nord"));
    }

    /// The call sites must use the constants, not re-introduce the old brand-like literals.
    #[test]
    fn call_sites_use_the_constants() {
        let ui = include_str!("library/ui.rs");
        assert!(ui.contains("branding::LIBRARY_TITLE"));
        assert!(!ui.contains("\"Nord Stage 4 Backup & Sample Library\""));
        let dlg = include_str!("debug_dialog.rs");
        assert!(dlg.contains("branding::BROWSER_CARD_TITLE"));
        assert!(dlg.contains("branding::COMPUTER_KB_CARD_DESC"));
        let panel = include_str!("panel/mod.rs");
        assert!(panel.contains("branding::CHASSIS_WORDMARK"));
        assert!(!panel.contains("\"nord stage 4\""));
        let main = include_str!("main.rs");
        assert!(main.contains("branding::PRODUCT_NAME"));
        assert!(!main.contains("\"Nord Stage 4\","));
    }

    #[test]
    fn exact_labels_are_pinned() {
        assert_eq!(
            LIBRARY_TITLE,
            "Backup & Sample Library (Nord Stage 4 files)"
        );
        assert_eq!(
            BACKUP_SOUND_FILTER_LABEL,
            "Backup and sound files for Nord Stage 4"
        );
    }
}
