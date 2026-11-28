//! Visual identity guard (D251): the default look must stay a neutral, non-red ND Matrix 1 design.
//!
//! These tests read the shipped palette tokens and the embedded baked plate, so a re-bake or a
//! palette edit that brings back a red chassis fails here. Spec:
//! `docs/specs/visual-style-and-panel.md` section 2.

use crate::baked::{BAKED_PLATE_2X_PNG, BAKED_SPRITES_2X_PNG};
use crate::style::*;
use egui::Color32;

/// A colour is "red dominant" when its red channel leads both others by 1.8x (the old chassis
/// red is 4.6x; the amber header accent is 1.4x and does not count).
fn red_dominant(r: u8, g: u8, b: u8) -> bool {
    r > 90 && f32::from(r) > 1.8 * f32::from(g.max(b))
}

fn token_red_dominant(c: Color32) -> bool {
    red_dominant(c.r(), c.g(), c.b())
}

/// Share of opaque pixels of an embedded PNG that are red dominant.
fn red_fraction(png: &[u8]) -> f32 {
    let img = image::load_from_memory(png).expect("valid PNG").to_rgba8();
    let (mut red, mut opaque) = (0_u64, 0_u64);
    for p in img.pixels() {
        if p[3] > 200 {
            opaque += 1;
            if red_dominant(p[0], p[1], p[2]) {
                red += 1;
            }
        }
    }
    assert!(opaque > 0, "PNG has no opaque pixels");
    red as f32 / opaque as f32
}

#[test]
fn surface_tokens_are_not_red() {
    for (name, c) in [
        ("CHASSIS_BASE", CHASSIS_BASE),
        ("CHASSIS_DARK", CHASSIS_DARK),
        ("MODULE_SLATE", MODULE_SLATE),
        ("MODULE_SLATE_DARK", MODULE_SLATE_DARK),
        ("MODULE_HEADER_BAR", MODULE_HEADER_BAR),
        ("HEADER_DARK", HEADER_DARK),
        ("PROGRAM_PLATE", PROGRAM_PLATE),
        ("BUTTON_GREY", BUTTON_GREY),
        ("BUTTON_BLACK", BUTTON_BLACK),
        ("BUTTON_ACCENT", BUTTON_ACCENT),
        ("DRAWBAR_WHITE", DRAWBAR_WHITE),
        ("DRAWBAR_BLACK", DRAWBAR_BLACK),
        ("KEY_WHITE", KEY_WHITE),
        ("KEY_WHITE_PRESSED", KEY_WHITE_PRESSED),
        ("LED_ACCENT", LED_ACCENT),
    ] {
        assert!(!token_red_dominant(c), "{name} is red dominant: {c:?}");
    }
}

#[test]
fn chassis_is_a_dark_cool_surface() {
    // Blue leads red on the chassis family, so the panel reads navy/graphite, not warm.
    for c in [
        CHASSIS_BASE,
        CHASSIS_DARK,
        MODULE_SLATE,
        MODULE_SLATE_DARK,
        HEADER_DARK,
    ] {
        assert!(c.b() > c.r(), "{c:?} is not cool");
        assert!(
            contrast_ratio(c, Color32::WHITE) > 10.0,
            "{c:?} is not dark"
        );
    }
}

#[test]
fn baked_plate_and_sprites_have_no_red_chassis() {
    let plate = red_fraction(BAKED_PLATE_2X_PNG);
    assert!(plate < 0.002, "plate red fraction {plate}");
    let sprites = red_fraction(BAKED_SPRITES_2X_PNG);
    assert!(sprites < 0.002, "sprite atlas red fraction {sprites}");
}

#[test]
fn red_detector_flags_the_old_chassis_and_passes_the_new_one() {
    // The old chassis red (#8E1B1F) must be caught, or the guard above proves nothing.
    assert!(red_dominant(0x8E, 0x1B, 0x1F));
    assert!(!red_dominant(0x15, 0x1C, 0x29));
}
