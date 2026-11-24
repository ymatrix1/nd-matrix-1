---
# ns4-2v7q
title: 'GUI text size option for readability (issue #592)'
status: completed
type: task
priority: high
created_at: 2026-11-24T02:05:15Z
updated_at: 2026-11-24T02:05:15Z
parent: ns4-xc5n
---

## Context
Issue ymatrix1/nd-matrix-1#592 ("Gui Text"): a user with poor eyesight cannot read the panel settings and asks for larger text. The owner replied they plan a config option for a double-stacked layout dividing the controls into two horizontal sections.

## Higher Goal
The panel is readable by users with low vision, in the standalone app and the DAW plugin editor.

## Acceptance Criteria
- [x] Text scale option (for example 100 to 200 percent) in the standalone GUI and plugin editor, persisted in the app settings, applied to panel labels, OLED-adjacent text, menus and dialogs
- [x] Layout stays usable at the largest scale (no clipped or overlapping labels); headless text-rectangle checks at 100 to 200 percent (no golden bitmap: the headless context has no GPU)
- [x] Double-stacked layout option (controls in two horizontal sections) designed in docs/specs/visual-style-and-panel.md section 5a; implementation split into bean `ns4-ojc9` (needs a `PlateMap` through `baked_view.rs`, which `ns4-80xe` is rebaking around)
- [x] Spec, README GUI section and docs/ROADMAP.md updated
- [ ] Issue #592 answered with the release that ships it (reply drafted in the PR, to be posted by the owner)

## Out of Scope
Changing the panel palette or baked art (ns4-80xe).

## Summary of Changes
- `crates/nord-gui/src/text_scale.rs`: `TextScale` (100 to 200 %, steps of 25), `px()` / `factor()` / `bar_height()` (thread-local factor), `scale_style`, `apply`, `handle_keys` (`Ctrl +` / `-` / `0`), `bar_controls` (pills `A-` `NNN%` `A+`), `load` / `save` (key `text_scale_percent` in `ns4/config.json`, other saved settings kept; `firmware_setting::set` / `clear` keep it too).
- `BakedZoom::base` (`panel/baked_zoom.rs`): the `Fit` view of the plate is the factor times the fitted plate, from the top left.
- Standalone (`main.rs`) and plugin editor (`editor/draw.rs`, `top_bar.rs`): load at open, save on change, scaled bar height, minimum window size and plugin window aspect; explicit font sizes and fixed widths of the dialogs, firmware bar, library and OS update window go through `px()`.
- Tests: user config round trip and preservation (core), `text_scale.rs` units, `baked_zoom_base_tests.rs`, `nord-plugin/src/editor/text_scale_tests.rs` (bar labels whole and apart at 100 to 200 %, measured `DEBUG` pill text height, dialog tabs fit, plate magnified, keys save).
- Docs: spec 5a (double-stacked layout design) and 5b, D244, README, ROADMAP B13d / B13e, commands guide and Help table, gui-testing, plugin-editor. Follow-up bean `ns4-ojc9` for the stacked layout.
- Not verified: a real display and a DAW host (headless checks only).
- Review round (PR #658): standalone minimum window clamped to the monitor and sized from the measured bar (`min_window_width`), plugin editor asks the host for a matching window on a size change or at open, Studio Settings theme spacing scaled, `Ctrl +/-` ignored while typing, a malformed `text_scale_percent` reads as absent and keeps the other settings, test hygiene (factor reset, non-vacuous dialog check). Checks are text-rectangle checks, not golden bitmaps.
