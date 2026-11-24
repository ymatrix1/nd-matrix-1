//! ND Matrix 1 front-panel GUI library.
//!
//! Spec: `docs/specs/visual-style-and-panel.md`

pub mod audio;
pub mod baked;
pub mod branding;
pub mod debug_dialog;
pub mod file_picker;
pub mod firmware_panel;
pub mod headless_load;
pub mod help_commands;
pub mod library;
pub mod os_update_panel;
pub mod panel;
pub mod screenshot;
pub mod style;
pub mod telemetry;
pub mod text_scale;
pub mod widgets;

#[cfg(test)]
mod identity_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gui_palette_colors() {
        assert_eq!(style::CHASSIS_BASE.to_hex(), "#151c29ff");
        assert_eq!(style::MODULE_SLATE.to_hex(), "#243249ff");
        assert_eq!(style::OLED_BG.to_hex(), "#06090eff");
    }
}
