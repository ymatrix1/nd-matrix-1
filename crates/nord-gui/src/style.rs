//! ND Matrix 1 "Slate Matrix" surface colours and tokens.
//!
//! Original palette (D134): graphite-navy chassis, bone plates, one cyan accent, and one accent
//! hue per section. The baked plate (`baked.rs`, built from `prototypes/panel-mock/`) uses the
//! same values; the vector fallback panel and the plugin chrome draw from these tokens.
//!
//! Provenance: every colour here is `ASSUMED` (a design choice, not a measurement of any
//! hardware); the only checked property is text contrast (`LEGEND_MIN_CONTRAST`, tests below).
//!
//! Spec: `docs/specs/visual-style-and-panel.md`

use egui::Color32;

/// Graphite-navy chassis base (`#151C29`).
pub const CHASSIS_BASE: Color32 = Color32::from_rgb(0x15, 0x1C, 0x29);

/// Darker recessed chassis seam and plugin bar fill (`#0B1018`).
pub const CHASSIS_DARK: Color32 = Color32::from_rgb(0x0B, 0x10, 0x18);

/// Accent fill of the highlighted (store / arp run) buttons (`#35D9E1`).
pub const BUTTON_ACCENT: Color32 = Color32::from_rgb(0x35, 0xD9, 0xE1);

/// Slate-navy module background (`#243249`).
pub const MODULE_SLATE: Color32 = Color32::from_rgb(0x24, 0x32, 0x49);

/// Darker navy for recessed frames (`#1A2538`).
pub const MODULE_SLATE_DARK: Color32 = Color32::from_rgb(0x1A, 0x25, 0x38);

/// Bone plate used for light tabs and sub-panels (`#D3CCB9`).
pub const MODULE_HEADER_BAR: Color32 = Color32::from_rgb(0xD3, 0xCC, 0xB9);

/// Dark section header (plate rim and tab, `#0D121B`); light header text ([`HEADER_INK`]) reads
/// at more than 15:1.
pub const HEADER_DARK: Color32 = Color32::from_rgb(0x0D, 0x12, 0x1B);

/// Header title and caption ink on [`HEADER_DARK`] (`#E8ECF0`).
pub const HEADER_INK: Color32 = Color32::from_rgb(0xE8, 0xEC, 0xF0);

/// Hairline edge of the dark header (`#2B3A52`).
pub const HEADER_EDGE: Color32 = Color32::from_rgb(0x2B, 0x3A, 0x52);

/// Silkscreen group outline border, steel blue (`#7F97BA`).
pub const GROUP_BORDER: Color32 = Color32::from_rgb(0x7F, 0x97, 0xBA);

/// Bone rectangular layer button (`#CFC8B5`).
pub const BUTTON_GREY: Color32 = Color32::from_rgb(0xCF, 0xC8, 0xB5);

/// Graphite selector button (`#1B2231`).
pub const BUTTON_BLACK: Color32 = Color32::from_rgb(0x1B, 0x22, 0x31);

/// Recessed button bezel dark border (`#05080D`).
pub const BUTTON_BEZEL: Color32 = Color32::from_rgb(0x05, 0x08, 0x0D);

/// Active-state accent LED, cyan (`#35E0E8`).
pub const LED_ACCENT: Color32 = Color32::from_rgb(0x35, 0xE0, 0xE8);

/// Dim unlit state for accent LEDs (`#0B3A3E`).
pub const LED_ACCENT_DIM: Color32 = Color32::from_rgb(0x0B, 0x3A, 0x3E);

/// 11-segment level ladder and split active indicator LED, mint (`#3DE8A8`).
pub const LED_GREEN: Color32 = Color32::from_rgb(0x3D, 0xE8, 0xA8);

/// Dim unlit state for mint LEDs (`#0A3A2A`).
pub const LED_GREEN_DIM: Color32 = Color32::from_rgb(0x0A, 0x3A, 0x2A);

/// Amber Morph assignment LED (`#FFB020`).
pub const LED_AMBER: Color32 = Color32::from_rgb(0xFF, 0xB0, 0x20);

/// Dim unlit state for amber LEDs (`#45300A`).
pub const LED_AMBER_DIM: Color32 = Color32::from_rgb(0x45, 0x30, 0x0A);

/// Unlit ladder segment and LED well (`#2A3344`).
pub const LED_OFF_WELL: Color32 = Color32::from_rgb(0x2A, 0x33, 0x44);

/// OLED display background (`#06090E`).
pub const OLED_BG: Color32 = Color32::from_rgb(0x06, 0x09, 0x0E);

/// OLED pixel graphic and text colour, pale cyan (`#BFF6F2`).
pub const OLED_FG: Color32 = Color32::from_rgb(0xBF, 0xF6, 0xF2);

/// Sand harmonic drawbar cap (`#E6DCC0`).
pub const DRAWBAR_WHITE: Color32 = Color32::from_rgb(0xE6, 0xDC, 0xC0);

/// Navy sub/mutation drawbar cap (`#1F2A3F`).
pub const DRAWBAR_BLACK: Color32 = Color32::from_rgb(0x1F, 0x2A, 0x3F);

/// Plate behind the Program / Master blocks and their legends (`#16202F`).
pub const PROGRAM_PLATE: Color32 = Color32::from_rgb(0x16, 0x20, 0x2F);

/// Ivory white key face (`#F7F1E0`); pressed keys use [`KEY_WHITE_PRESSED`].
pub const KEY_WHITE: Color32 = Color32::from_rgb(0xF7, 0xF1, 0xE0);

/// Pressed white key tint, pale teal (`#D3EFE9`).
pub const KEY_WHITE_PRESSED: Color32 = Color32::from_rgb(0xD3, 0xEF, 0xE9);

/// Smallest contrast ratio (WCAG 2, 4.5:1 for small text) a printed legend keeps against the panel,
/// dimmed or not.
pub const LEGEND_MIN_CONTRAST: f32 = 4.5;

fn linear(c: u8) -> f32 {
    let v = f32::from(c) / 255.0;
    if v <= 0.039_28 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: Color32) -> f32 {
    0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
}

/// WCAG 2 contrast ratio between two opaque colours (1 to 21).
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Colour of a dimmed (not supported yet, or secondary) legend: `ink` faded toward the panel, but
/// never below [`LEGEND_MIN_CONTRAST`] against the slate and the program plate, so a dimmed
/// legend stays readable. The fade is strongest at a blend of 0.45.
pub fn dim_legend(ink: Color32) -> Color32 {
    let lerp = |bg: Color32, t: f32| {
        let m = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Color32::from_rgb(m(bg.r(), ink.r()), m(bg.g(), ink.g()), m(bg.b(), ink.b()))
    };
    let mut t = 0.45_f32;
    while t < 1.0 {
        let c = lerp(MODULE_SLATE, t);
        if contrast_ratio(c, MODULE_SLATE) >= LEGEND_MIN_CONTRAST
            && contrast_ratio(lerp(PROGRAM_PLATE, t), PROGRAM_PLATE) >= LEGEND_MIN_CONTRAST
        {
            return c;
        }
        t += 0.01;
    }
    ink
}

/// Configure egui visual style to use our own chassis colours (`CHASSIS_BASE`, `MODULE_SLATE`, `OLED_BG`); the panel art itself is the baked plate (`baked.rs`).
pub fn setup_nord_visuals(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = CHASSIS_BASE;
    visuals.window_fill = MODULE_SLATE;
    visuals.extreme_bg_color = OLED_BG;
    ctx.set_visuals(visuals);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_tokens() {
        assert_eq!(CHASSIS_BASE.r(), 0x15);
        assert_eq!(MODULE_SLATE.g(), 0x32);
        assert_eq!(LED_GREEN.b(), 0xA8);
        assert_eq!(BUTTON_ACCENT.r(), 0x35);
    }

    #[test]
    fn contrast_ratio_matches_wcag_reference_values() {
        assert!((contrast_ratio(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn dimmed_legends_keep_the_minimum_contrast_on_slate_and_program_plate() {
        for ink in [
            Color32::WHITE,
            Color32::from_gray(225),
            Color32::from_rgb(235, 190, 190),
        ] {
            let c = dim_legend(ink);
            for bg in [MODULE_SLATE, PROGRAM_PLATE] {
                assert!(
                    contrast_ratio(c, bg) >= LEGEND_MIN_CONTRAST,
                    "{c:?} on {bg:?}"
                );
            }
            // Still dimmer than the full ink.
            assert!(luminance(c) < luminance(ink));
        }
        // The legacy greys (150 and 105) were below the floor.
        assert!(contrast_ratio(Color32::from_gray(105), MODULE_SLATE) < LEGEND_MIN_CONTRAST);
    }
}
