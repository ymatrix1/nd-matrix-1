//! Pieces of the Program home screen shared by its views: the location title with the boxed `E`
//! marker, the `Live N` title of Live Mode and the per-section rows.

use super::oled::Summary;
use super::oled_icons::{is_icon, ICON_ORGAN, ICON_PIANO, ICON_SYNTH, ICON_WARN};
use crate::panel::ui_state::ProgramUi;

/// Marks the boxed `E` inside a text line. The painter draws a box with an `E` in its place;
/// [`strip_marks`] turns it into plain text for the tooltip.
///
/// ASSUMED: a private-use character as in-band marker (no real text uses it), chosen so
/// `OledText` keeps its shape for every screen.
pub const EDITED: char = '\u{E0E1}';

/// Plain-text form of the boxed `E` (tooltip).
const EDITED_PLAIN: &str = " [E]";

/// `text` with every [`EDITED`] mark written out and the row icons dropped, for plain text (the
/// warning icon becomes `!`).
pub fn strip_marks(text: &str) -> String {
    text.replace(EDITED, EDITED_PLAIN)
        .replace(ICON_WARN, "!")
        .chars()
        .filter(|&c| !is_icon(c))
        .collect()
}

/// Big title of the home screen: the location (`A:11`) followed by the boxed `E` while the
/// program is edited, or `Live N` in Live Mode.
///
/// VERIFIED: Live Mode shows `Live 1` as the big title (DSoundman course, Bank Games 19:45).
/// ASSUMED: Live Mode shows no `E` because Live programs store their edits by themselves
/// (manual p.13); footage of an edited Live program would confirm.
pub fn location_title(ui: &ProgramUi) -> String {
    if let Some(slot) = ui.live.slot() {
        return format!("Live {}", slot + 1);
    }
    let mut t = ui.loc.label();
    if ui.is_edited() {
        t.push(EDITED);
    }
    t
}

/// Program name for the line under the title; empty when it only repeats `Live N`.
pub fn name_line(ui: &ProgramUi) -> String {
    if ui.live.is_active() && ui.title.starts_with("Live ") {
        return String::new();
    }
    ui.title.clone()
}

/// Drawbar positions as digits, 16' first (`888000000`); values above 8 print as 8.
pub fn bars_text(bars: &[u8; 9]) -> String {
    bars.iter().map(|&b| char::from(b'0' + b.min(8))).collect()
}

/// One row per section that has a layer on: an icon marker, then the source name as the
/// hardware prints it (`B3`, `Pure Saw`; not `Organ B3`).
///
/// Organ rows end with the drawbar positions as nine digits (`888000000`, B3 only) and Piano rows
/// with the size letter of the loaded library file (`Pearl Upright  L`); both have a data source
/// (`organ_drawbars`, the `.npno` header name, see `nord_core::piano::label`). A piano with no
/// loaded sound shows the warning icon and `Piano not found`.
///
/// VERIFIED: the icon column, source names and the `Piano not found` warning row of the real home
/// screen (`tools/arm_screens.py`, `docs/research/arm-oled-render.md` section 12). ASSUMED: the
/// text form of the mini drawbars (hardware draws small bars) and the two-space gap before the
/// size letter (course footage PL 11:25, BG 17:10, SG 8:40, ORG 5:30). The preset id (`1:05`) the
/// hardware prints before a preset name has no source: a program stores no preset link, and the
/// emulator loads models rather than `.ns4o` / `.ns4n` / `.ns4y` presets (ns4-bjvn).
pub fn section_rows(sum: &Summary) -> Vec<String> {
    let fw = nord_core::oled_font::font(nord_core::oled_font::BAR_FONT_INDEX).is_some();
    section_rows_with(sum, fw)
}

/// Firmware glyph (`0x8A` of the 8 px font) of the missing-sound warning.
///
/// VERIFIED: the home screen's piano row text is `\x8aPiano not found`
/// (`tools/arm_screens.py press ... --key 0x01,0,3`); the glyph is drawn inside the text, after
/// the row's icon.
pub const WARN_GLYPH: char = '\u{8a}';

/// Drawbar bar-font characters of `bars` (`0..=8`), 16' first.
///
/// VERIFIED: the firmware's text for the default organ is `88AA09009`; the characters `0`..`8` are
/// bars of height 1..9 and `9`..`A` the same heights in the second style. ASSUMED: the second
/// style belongs to the white drawbars (positions 3, 4, 6 and 9: 8', 4', 2', 1'), the only
/// assignment under which that text reads as the default program's drawbars.
pub fn bar_chars(bars: &[u8; 9]) -> String {
    bars.iter()
        .enumerate()
        .map(|(i, &v)| {
            let white = matches!(i, 2 | 3 | 5 | 8);
            char::from(if white { b'9' } else { b'0' } + v.min(8))
        })
        .collect()
}

/// [`section_rows`] with the firmware fonts present (`fw`: glyph warning, drawbars drawn as bars,
/// size letter at the right edge) or not (text stand-ins).
pub fn section_rows_with(sum: &Summary, fw: bool) -> Vec<String> {
    let on = |a: usize| sum.layers_on[a] || sum.layers_on[a + 1];
    let mut rows = Vec::new();
    if on(0) {
        rows.push(match sum.organ_bars {
            Some(bars) if !fw => format!("{ICON_ORGAN}{} {}", sum.organ, bars_text(&bars)),
            _ => format!("{ICON_ORGAN}{}", sum.organ),
        });
    }
    if on(2) {
        rows.push(if sum.piano.is_empty() && fw {
            format!("{ICON_PIANO}{WARN_GLYPH}Piano not found")
        } else if sum.piano.is_empty() {
            format!("{ICON_WARN}Piano not found")
        } else {
            match sum.piano_size {
                // the size letter sits at the right edge of the row (tab = right-aligned part)
                Some(size) => format!("{ICON_PIANO}{}\t{}", sum.piano, size.letter()),
                None => format!("{ICON_PIANO}{}", sum.piano),
            }
        });
    }
    if on(4) || sum.layers_on[6] {
        rows.push(format!("{ICON_SYNTH}{}", sum.synth));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_shows_the_location_then_the_edited_mark() {
        let mut u = ProgramUi::default();
        assert_eq!(location_title(&u), "A:11");
        u.sync_edits(1);
        assert_eq!(location_title(&u), format!("A:11{EDITED}"));
        assert_eq!(strip_marks(&location_title(&u)), "A:11 [E]");
    }

    #[test]
    fn live_mode_titles_the_slot_and_drops_the_mark() {
        let mut u = ProgramUi::default();
        u.sync_edits(1);
        u.live.select(1);
        u.title = "Live 1".into();
        assert_eq!(location_title(&u), "Live 1");
        assert_eq!(name_line(&u), "");
        u.title = "Pad".into();
        assert_eq!(name_line(&u), "Pad");
    }

    #[test]
    fn rows_list_only_sections_with_a_layer_on() {
        let sum = Summary {
            organ: "B3".into(),
            piano: "Grand".into(),
            synth: "Saw".into(),
            layers_on: [true, false, false, false, false, false, true],
            ..Default::default()
        };
        assert_eq!(
            section_rows_with(&sum, false),
            [format!("{ICON_ORGAN}B3"), format!("{ICON_SYNTH}Saw")]
        );
        assert!(section_rows_with(&Summary::default(), false).is_empty());
    }

    #[test]
    fn organ_rows_show_the_drawbars_and_piano_rows_the_size_letter() {
        let sum = Summary {
            organ: "B3".into(),
            piano: "Pearl Upright".into(),
            piano_size: Some(nord_core::piano::label::PianoSize::Large),
            organ_bars: Some([8, 8, 8, 0, 0, 0, 0, 0, 0]),
            layers_on: [true, false, true, false, false, false, false],
            ..Default::default()
        };
        assert_eq!(
            section_rows_with(&sum, false),
            [
                format!("{ICON_ORGAN}B3 888000000"),
                format!("{ICON_PIANO}Pearl Upright	L")
            ]
        );
        assert_eq!(
            strip_marks(&section_rows_with(&sum, false)[0]),
            "B3 888000000"
        );
        assert_eq!(bars_text(&[0, 1, 2, 3, 4, 5, 6, 7, 8]), "012345678");
        assert_eq!(bars_text(&[9; 9]), "888888888");
    }

    #[test]
    fn with_firmware_fonts_the_drawbars_become_bars_and_the_warning_a_glyph() {
        let sum = Summary {
            organ: "B3".into(),
            organ_bars: Some([8, 8, 8, 8, 0, 0, 0, 0, 0]),
            layers_on: [true, false, true, false, false, false, false],
            ..Default::default()
        };
        let rows = section_rows_with(&sum, true);
        assert_eq!(
            rows[0],
            format!("{ICON_ORGAN}B3"),
            "bars are drawn, not printed"
        );
        assert_eq!(rows[1], format!("{ICON_PIANO}{WARN_GLYPH}Piano not found"));
        // the firmware's own text for the default program
        assert_eq!(bar_chars(&[8, 8, 8, 8, 0, 0, 0, 0, 0]), "88AA09009");
        assert_eq!(bar_chars(&[8; 9]), "88AA8A88A");
    }

    #[test]
    fn a_piano_without_a_sound_shows_the_warning_row() {
        let sum = Summary {
            layers_on: [false, false, true, false, false, false, false],
            ..Default::default()
        };
        let rows = section_rows_with(&sum, false);
        assert_eq!(rows, [format!("{ICON_WARN}Piano not found")]);
        assert_eq!(strip_marks(&rows[0]), "!Piano not found");
    }
}
