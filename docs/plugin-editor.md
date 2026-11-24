# Plugin editor: build and host smoke test

The CLAP/VST3 plugin has an egui editor (`crates/nord-plugin/src/editor/`, spec: `docs/specs/daw-plugin.md`, section Editor). It was verified by unit and headless frame tests only. **It was loaded by hand in FL Studio on Windows (2026-11-22, top bar and program load only, `docs/HOST_COMPATIBILITY.md` 7.9); the full smoke test below has not been run.** Run it in Reaper and Bitwig before ticking the host item of B11 (`ns4-hud1`) and B13.

## Build

```bash
cargo xtask bundle nord-plugin --release
```

Bundles land in `target/bundled/` (`nd-matrix-1.clap`, `nd-matrix-1.vst3`). Copy them to the host's plugin folder (Windows: `%COMMONPROGRAMFILES%\CLAP`, `%COMMONPROGRAMFILES%\VST3`; macOS and Linux: the usual user folders). Quick check without a host: `cargo build -p nord-plugin --release`.

## Smoke test (Reaper, Bitwig)

1. Rescan plugins; add "Nord Stage 4" to an instrument track. Expect: no crash, the editor opens and shows the red panel with the keybed.
2. Click keys on the on-screen keybed: Organ A sounds. Hold and release a key: no stuck note.
3. Drag the Master Level knob and an Organ drawbar. In the host's parameter list `master_level` and `drawbar_*` move; arm write automation, drag again, play back: the knob follows the lane.
4. Move a parameter from the host (automation lane or generic UI): the panel knob follows within a frame.
5. Hover a dimmed control: tooltip "not automatable in the plugin yet".
6. Drag the window corner: the panel scales and stays centred; close and reopen: the size is kept; save and reload the project: the size is kept.
7. Change the host's DPI scaling (or move the window to a HiDPI display): the panel stays sharp and fully visible.
8. Click `OPEN .NS4B`: the Library tab opens; pick a `.ns4b`, then a program: it loads and a key sounds with the program's patch (D218). The host UI stays responsive while the file dialog is open.
9. Click `DEBUG` and `? HELP`: the diagnostics dialog and the Setup Guide open inside the editor window and close again.
10. After a program load, move one knob of a group: that group follows the host again, other groups keep the program. Reload the project: the program is gone (TD-21).
11. Record the host name, version, OS and the results in `docs/research/` with the date.

## What the editor automates

See the Editor section of `docs/specs/daw-plugin.md`. Of the 141 inventory controls bound to a host id (274 controls total, after merging main at the Synth/Effects rewire #221), 129 are live, 9 are Shift/Hold functions that share a rectangle with a live control (not separately automatable), and 3 are dimmed (`organ.zone_leds`, `piano.zone_leds`, `program.layer_scene_ii`).

### Menu items

The menus (SHIFT + Program 1..8 on the OLED) are automatable too. In the host's parameter list, `sys_*` (System, Sound, Output mono), `outx_*` (Output routing), `menx_*` (trigger point, SW B), `ped_*` / `pedx_*` (Pedal), `midix_*` (MIDI, Extern) and `auxkb_*` follow the OLED pages. Extra smoke steps for B11 / B13:

1. Open System, turn the dial on Global Transpose: `sys_transpose` moves in the host list; arm write automation, edit again, play back: the OLED value follows the lane.
2. Automate `pedx_tp1_left` or `ped_foot_func` from the host: Pedal page 3 / 6 shows the matching label.
3. Items with no host parameter (Local Control, MIDI channels, Extern bank / program / CC) stay on the panel only; the list and reasons are in `crates/nord-plugin/src/editor/menu_ui_only.rs` (`UI_ONLY`).
4. The panel opens with Sustain Pedal Type on SP-2: the engine and host default to it (continuous pedal), unlike the hardware's SP-1.

## Text size (issue #592)

The editor has the same text size option as the standalone app: `Ctrl +` / `Ctrl -` / `Ctrl 0` (some hosts keep these keys for themselves) and the `A-` `NNN%` `A+` pills in the top bar, 100 to 200 %. The size is read from `ns4/config.json` (`text_scale_percent`) when the editor opens and written on every change, so a new instance opens at the last size; it is a per-user setting, not part of the plugin state or a host parameter. It scales the top bar and the Studio Settings window, makes the top bar taller (`size_for_width` adds the scaled height and a minimum width that holds the bar, the aspect lock is unchanged); a new size, or a saved size at open, asks the host for the matching window (same width; the host may refuse)  and magnifies the plate: the `FIT` view shows it at the chosen factor from the left edge, and the usual pan, `+` / `-` and `1`-`5` move around. egui's zoom factor is not used (`egui_baseview` ignores it for layout). Tests: `crates/nord-plugin/src/editor/text_scale_tests.rs`. Not checked in a DAW host.
