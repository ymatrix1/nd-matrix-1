---
# ns4-op1w
title: Port nd matrix 1 HTML/CSS prototype visuals to native nord-panel / nord-gui (baker + egui widgets)
status: todo
type: task
created_at: 2026-11-19T18:52:52Z
updated_at: 2026-11-19T18:52:52Z
---

## Context

PR #533 (\
s4-gnvr\) established the approved 1:1 HTML/CSS hardware front-panel visual reference (\
d matrix 1\) in \prototypes/panel-mock/\. The native standalone app (\crates/nord-gui\) and DAW plugin (\crates/nord-plugin\, CLAP/VST3 via \
ih_plug_egui\) currently render an earlier 2D painter style. We want the native OpenGL/\gui\ UI to match the HTML/CSS prototype ~99% pixel-for-pixel while staying fast, lightweight, and single-source-of-truth inside DAWs (D15).

## Higher Goal

Deliver an automated, deterministic HTML/CSS-to-\gui\ baking pipeline plus updated \
ord-gui\ / \
ord-panel\ widget painters so visual edits made in \prototypes/panel-mock/\ port cleanly into both the standalone app and CLAP/VST3 plugin without embedding a WebView.

## Phased Porting Plan

1. **Phase 1 — Control ID Annotation & Automated Baker (\	ools/bake_panel_from_html.py\):**
   - Annotate every interactive control in \prototypes/panel-mock/panel.js\ with canonical \data-control-id\ attributes matching \crates/nord-panel/src/inventory/\.
   - Add a headless-Chromium (Playwright) baker script that renders at 2x HiDPI:
     - \panel_plate_2x.png\ (static deck: continuous red powder-coat metal, full-height wooden end cheeks, seamless front roll, \ = 6\ S-curve headers, transparent-title gapped group borders, silkscreen, knob scales, and \
d matrix 1\ wordmark — with moving caps/LEDs/OLEDs hidden via \.bake-static\).
     - Component sprite atlas (\panel_sprites_2x.png\): stadium-pill buttons (black/grey/red, up/down), 12-flute knob cap + pointer, stepped L-profile drawbars (white/black), tapered fader cap, pebbled stone Mod Wheel, wooden Pitch Stick, and grounded Fatar black/white key states.
     - \panel_layout.json\: normalized \(x, y, w, h)\ rects extracted via \getBoundingClientRect()\ for every \data-control-id\, LED, OLED bezel, and key.
   - Commit deterministic outputs with SHA-256 content hashes (D15) so CI never requires Chromium.
2. **Phase 2 — Surface Tokens & Baked Plate Integration (\crates/nord-gui/src/style.rs\, \widgets/plate.rs\):**
   - Sync color tokens in \style.rs\ and \docs/specs/visual-style-and-panel.md\ with \prototypes/panel-mock/styles.css\.
   - Load \panel_plate_2x.png\ and \panel_sprites_2x.png\ as \gui::TextureHandle\s once at startup.
3. **Phase 3 — Dynamic Widgets & Hardware Effects (\crates/nord-gui/src/widgets/\, \panel/\):**
   - **Buttons (\asic.rs\):** Draw up/down stadium-pill sprites + subtle hover highlight + flush crimson collars.
   - **Knobs & Encoders (\knob.rs\, \scale.rs\):** Fixed top-left lighting with rotating 12-flute pointer, tapered \MASTER LEVEL\ crescent, silver-blue encoder rings, and dark inlay arches.
   - **Faders & 9 Drawbars (\aders.rs\, \organ_drawbars.rs\):** Slide baked caps along tracks + live 8-LED red and 11-LED green ladders.
   - **Pitch Stick, Mod Wheel & 88-Key Keybed (\wheels.rs\, \keybed.rs\):** Spring-return Pitch Stick, upward-rolling pebbled Mod Wheel UV + dimple, and grounded Fatar black/white key states.
   - **LEDs & OLEDs (\display.rs\, \ctl_led_tag.rs\):** Radial glow overlays for round/triangular/split-lip LEDs and live monochrome OLED rendering over the baked bezels.
4. **Phase 4 — Verification & Spec Sync:**
   - Compare \
ord-gui\ \screenshot.rs\ output side-by-side against \prototypes/panel-mock/\ across all 6 bays.
   - Pass \legibility_tests.rs\, \rame_cost_tests.rs\, and all workspace CI checks (\cargo fmt/clippy/test\, \uff\, \pytest\, \check_tests.py\).
   - Update \docs/specs/visual-style-and-panel.md\ and \docs/DECISIONS.md\ (amending D15 if HTML/CSS replaces Blender as the offline baker).

## Acceptance Criteria

- [ ] \	ools/bake_panel_from_html.py\ deterministically exports \panel_plate_2x.png\, \panel_sprites_2x.png\, and \panel_layout.json\ from \prototypes/panel-mock/\ with unit tests in \	ools/tests/\.
- [ ] \crates/nord-gui\ renders the baked plate, sprites, LEDs, and OLEDs in both standalone (\frame\) and DAW plugin (\
ih_plug_egui\) targets.
- [ ] Visual diff against \prototypes/panel-mock/\ confirms 1:1 parity across all 6 bays and the 88-key keybed.
- [ ] \legibility_tests.rs\, \rame_cost_tests.rs\, and all Rust/Python CI gates pass.

## Out of Scope

- Embedding a live WebView inside \
ord-plugin\.
- Changing DSP synthesis behavior in \crates/nord-core\.

## Summary of Changes

- Pending implementation.
