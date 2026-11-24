# Commands and zoom

Every key, click and drag of `nd-matrix-1` (standalone) and the CLAP / VST3 plugin. The same table
is on the Help screen: `? HELP` in the top bar, "Setup Guide" tab, card 3. Source of both:
`crates/nord-gui/src/help_commands.rs`.

## Why zoom

The front panel is one wide picture (about 4:1). Fitted to a 1760 px window it shows at about half
size, so small text is hard to read. Zoom into one section, then fit again. Nothing changes in the
sound; zoom only moves the view.

## Text size (issue #592)

If the printed text is too small even when zoomed, make everything bigger: press `Ctrl +` (or
`Ctrl =`) or click `A+` in the top bar, and repeat up to 200 %. `Ctrl -` or `A-` go back one step;
`Ctrl 0`, or a click on the percentage between `A-` and `A+`, restores 100 %. The choice is saved per
user (`ns4/config.json`, key `text_scale_percent`) and used by the standalone and by the plugin
editor the next time they open.

It scales the top bar, menus, combo boxes, tooltips and the Studio Settings window, and magnifies the
picture of the panel by the same factor. `FIT` (and `0`) then shows the panel at 150 %, say, starting
at its left edge; move with a drag or the section keys `1`-`5`. The zoom pills show `150%` there,
because zoom is measured against the whole panel; `+` still zooms further. A layout that stacks the
controls in two rows, so the same window shows bigger print without panning, is designed in the
[visual style spec](../specs/visual-style-and-panel.md#5a-double-stacked-layout-design-issue-592)
and is not built yet.

## Step by step

1. **Start.** Run `nd-matrix-1`, or open the plugin editor. The whole panel is fitted (`100%`).
2. **Jump to a section.** Hover a section: it lights up and a tip says "Click to zoom". Click an empty part of it (knobs and buttons still edit), or press `1` for Organ, `2` Piano, `3` Program, `4` Synth, `5` Effects, or click
   `ORG PNO PRG SYN FX` in the top bar. The section fills the window.
3. **Zoom freely.** Press `+` / `-`, click the `+` / `-` pills, or hold `Ctrl` and scroll to zoom at the
   pointer. The percentage in the bar shows the zoom (`100%` is the whole panel).
4. **Move around.** Drag any empty part of the panel to pan. Knobs, buttons and keys still work
   while zoomed; dragging on a control edits it instead of panning.
5. **Come back.** Press `0` or `Esc` (Esc only while zoomed), or click `FIT`.

Zoom is clamped to 100%..600%. The top bar stays the same size and always lines up with the visible
part of the panel.

## Commands

| Group | Keys | Effect |
|---|---|---|
| Zoom | `1 2 3 4 5` | Zoom to Organ, Piano, Program, Synth, Effects |
| Zoom | `Click empty panel in a section` | Zoom to it (lit when hovered); click again for the whole panel |
| Zoom | `0 or Esc` | Whole panel (Esc only while zoomed) |
| Zoom | `+ / -` | Zoom in / out one step |
| Zoom | `Ctrl + scroll or pinch` | Zoom at the pointer |
| Zoom | `Drag empty plate` | Pan while zoomed |
| Zoom | `Bar: - 100% + FIT ORG PNO PRG SYN FX` | Same as the keys, with the click |
| Text size | `Ctrl + / Ctrl -` | Larger / smaller text, 100 to 200 % in steps of 25 (saved) |
| Text size | `Ctrl 0` | Back to 100 % |
| Text size | `Bar: A- 100% A+` | Same as the keys, with the click |
| Play | `A S D F G H J K L ; '` | White keys from the base octave (C4 = MIDI 60) |
| Play | `W E T Y U O P` | Black keys |
| Play | `Z / X` | Base octave down / up |
| Play | `C / V` | Velocity down / up by 8 |
| Play | `Space` | Sustain pedal |
| Mouse | `Click or drag the keybed` | Play; the hit position sets the velocity |
| Mouse | `Scroll over a knob, fader or drawbar` | Fine adjustment |
| Mouse | `Drag Pitch Stick / Mod Wheel` | Pitch bend / modulation |
| Window | `Drag the empty top strip` | Move the window |
| Window | `Drag an edge or corner` | Resize the window |
| Files | `Drop .ns4b .ns4p .npno .nsmp4` | Load onto the instrument |
| Files | `Shift + L` | Library drawer (Shift while the computer keyboard plays) |

Keys are physical positions, so they are the same on QWERTY and ABNT2. Digits, `0`, `+` and `-` never
play notes; the computer-keyboard playing keys are listed in [GUI testing](../gui-testing.md).

## Plugin notes

- The plugin window is sized by the host; the corner handle resizes it with the aspect locked.
  Zoom works inside that window like in the standalone.
- Some hosts keep the keyboard focus for themselves (the DAW shortcuts win). The top-bar pills and
  Ctrl + scroll always work.
- The Help screen is the same in the plugin (`? HELP`).

## Where it lives

| Part | File |
|---|---|
| Zoom state, keys, bar pills | `crates/nord-gui/src/panel/baked_zoom.rs` |
| Plate drawing (uses the zoomed rectangle) | `crates/nord-gui/src/panel/baked_view.rs` |
| Commands table (Help screen) | `crates/nord-gui/src/help_commands.rs` |
| Older focus / panorama view of the non-baked panel | `crates/nord-gui/src/panel/view.rs` |
