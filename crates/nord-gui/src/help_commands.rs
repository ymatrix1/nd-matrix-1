//! The commands table shown on the Help screen (`? HELP`, "Setup Guide" tab) and mirrored in
//! `docs/guides/commands.md`. One table so the screen and the guide cannot drift apart: a test
//! checks that every row's keys appear in the guide file.

use egui::{Color32, RichText, Ui};

/// One command: where it works, the keys or clicks, and what it does.
pub struct Command {
    pub group: &'static str,
    pub keys: &'static str,
    pub effect: &'static str,
}

const fn c(group: &'static str, keys: &'static str, effect: &'static str) -> Command {
    Command {
        group,
        keys,
        effect,
    }
}

/// Every command, grouped. Keys are physical positions (same on QWERTY and ABNT2).
pub const COMMANDS: &[Command] = &[
    c(
        "Zoom",
        "1 2 3 4 5",
        "Zoom to Organ, Piano, Program, Synth, Effects",
    ),
    c(
        "Zoom",
        "Click empty panel in a section",
        "Zoom to it (lit when hovered); click again for the whole panel",
    ),
    c("Zoom", "0 or Esc", "Whole panel (Esc only while zoomed)"),
    c("Zoom", "+ / -", "Zoom in / out one step"),
    c("Zoom", "Ctrl + scroll or pinch", "Zoom at the pointer"),
    c("Zoom", "Drag empty plate", "Pan while zoomed"),
    c(
        "Zoom",
        "Bar: - 100% + FIT ORG PNO PRG SYN FX",
        "Same as the keys, with the click",
    ),
    c(
        "Text size",
        "Ctrl + / Ctrl -",
        "Larger / smaller text, 100 to 200 % in steps of 25 (saved)",
    ),
    c("Text size", "Ctrl 0", "Back to 100 %"),
    c(
        "Text size",
        "Bar: A- 100% A+",
        "Same as the keys, with the click",
    ),
    c(
        "Play",
        "A S D F G H J K L ; '",
        "White keys from the base octave (C4 = MIDI 60)",
    ),
    c("Play", "W E T Y U O P", "Black keys"),
    c("Play", "Z / X", "Base octave down / up"),
    c("Play", "C / V", "Velocity down / up by 8"),
    c("Play", "Space", "Sustain pedal"),
    c(
        "Mouse",
        "Click or drag the keybed",
        "Play; the hit position sets the velocity",
    ),
    c(
        "Mouse",
        "Scroll over a knob, fader or drawbar",
        "Fine adjustment",
    ),
    c(
        "Mouse",
        "Drag Pitch Stick / Mod Wheel",
        "Pitch bend / modulation",
    ),
    c("Window", "Drag the empty top strip", "Move the window"),
    c("Window", "Drag an edge or corner", "Resize the window"),
    c(
        "Files",
        "Drop .ns4b .ns4p .npno .nsmp4",
        "Load onto the instrument",
    ),
    c(
        "Files",
        "Shift + L",
        "Library drawer (Shift while the computer keyboard plays)",
    ),
];

/// Draw the table as a grid, one block per group.
pub fn show(ui: &mut Ui, key_color: Color32, text_color: Color32) {
    let mut group = "";
    egui::Grid::new("help_commands")
        .num_columns(3)
        .spacing([14.0, 4.0])
        .show(ui, |ui| {
            for cmd in COMMANDS {
                let first = cmd.group != group;
                group = cmd.group;
                ui.label(
                    RichText::new(if first { cmd.group } else { "" })
                        .size(crate::text_scale::px(11.0))
                        .strong()
                        .color(text_color),
                );
                ui.label(
                    RichText::new(cmd.keys)
                        .size(crate::text_scale::px(11.0))
                        .monospace()
                        .color(key_color),
                );
                ui.label(
                    RichText::new(cmd.effect)
                        .size(crate::text_scale::px(11.0))
                        .color(text_color),
                );
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_complete_and_grouped() {
        assert!(COMMANDS.len() >= 15);
        let mut seen: Vec<&str> = Vec::new();
        for cmd in COMMANDS {
            assert!(!cmd.keys.is_empty() && !cmd.effect.is_empty());
            if seen.last() != Some(&cmd.group) {
                assert!(
                    !seen.contains(&cmd.group),
                    "{} split in two blocks",
                    cmd.group
                );
                seen.push(cmd.group);
            }
        }
    }

    #[test]
    fn the_guide_file_lists_every_command() {
        let guide = include_str!("../../../docs/guides/commands.md");
        for cmd in COMMANDS {
            assert!(
                guide.contains(cmd.keys),
                "docs/guides/commands.md misses `{}`",
                cmd.keys
            );
        }
    }

    #[test]
    fn the_table_draws() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, Color32::WHITE, Color32::GRAY));
        });
    }
}
