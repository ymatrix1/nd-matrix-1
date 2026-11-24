---
# ns4-ojc9
title: 'Double-stacked panel layout option (issue #592)'
status: todo
type: task
priority: normal
created_at: 2026-11-24T03:30:00Z
updated_at: 2026-11-24T03:30:00Z
parent: ns4-xc5n
blocked_by:
    - ns4-80xe
---

## Context
Issue ymatrix1/nd-matrix-1#592 ("Gui Text"): the owner announced a config option for a double-stacked layout that divides the controls into two horizontal sections. The text size option (`ns4-2v7q`, D244) shipped first and enlarges the print by showing less of the plate; this layout enlarges it by showing more of it at the same window size. Design: `docs/specs/visual-style-and-panel.md` section 5a (rows split at plate x = 1323, keybed below at 0.503x, canvas 1323 x 956, 1.4x to 2x bigger print at the same window).

## Higher Goal
Low-vision users read the panel in a normal-sized window without panning, in the standalone app and the DAW plugin editor.

## Acceptance Criteria
- [ ] `PlateMap` (three source/destination rectangle pairs; wide mode is the identity) replaces the single plate rectangle in `baked_view.rs` and `baked_buttons.rs`; wide mode output byte-identical to today
- [ ] `layout` = `wide` | `stacked` saved in `ns4/config.json`, bar pill and `Ctrl L`, standalone and plugin editor
- [ ] Zoom (`baked_zoom.rs`), hover, section keys 1-5, keybed hits and key lighting work in stacked mode; plugin window aspect follows the layout
- [ ] Tests: no control or bay rectangle of `panel_layout.json` crosses the split; wide rectangles unchanged; stacked rectangles inside the canvas, no overlap; headless 100 and 150 percent frame shows no clipped label in stacked mode (same check as `crates/nord-plugin/src/editor/text_scale_tests.rs`)
- [ ] Spec section 5a status updated, README GUI section, `docs/ROADMAP.md` and issue #592 answered with the release that ships it

## Out of Scope
Rebaking or recolouring the plate art (`ns4-80xe`), a third arrangement, per-section layouts.
