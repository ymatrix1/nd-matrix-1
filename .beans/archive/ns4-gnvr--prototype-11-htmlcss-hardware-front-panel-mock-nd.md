---
# ns4-gnvr
title: Prototype 1:1 HTML/CSS hardware front-panel mock (nd matrix 1)
status: completed
type: task
created_at: 2026-11-19T18:02:46Z
updated_at: 2026-11-19T18:02:46Z
---

## Context

Before porting refined 2.5D hardware panel visuals and component geometry into the native Rust \
ord-panel\ / \
ord-gui\ crates (D15), we built an interactive 1:1 HTML/CSS visual reference prototype in \prototypes/panel-mock/\ modeled from high-resolution hardware and component reference imagery.

## Higher Goal

Lock in exact 1:1 visual proportions, color tokens, powder-coat surface finish, component geometry, LED inventory, and spacing in a fast-iterating browser sandbox so it can serve as the visual specification (or offline baker source) for \
ord-panel\ and \
ord-gui\.

## Acceptance Criteria

- [x] Full 6-bay horizontal panel layout (\Master / Rotary\, \Organ\, \Piano\, \Program / Morph\, \Synth\, \Layer Effects\) plus 88-key Hammer Action keybed in \prototypes/panel-mock/\.
- [x] Authentic hardware component styling: stadium-pill buttons, 12-flute knobs with wrap-over white pointers, tapered \MASTER LEVEL\ crescent, stepped L-profile drawbars, tapered fader caps, pebbled stone Mod Wheel with concave dimple, wooden Pitch Stick, continuous wood-grain end cheeks, and grounded Fatar black keys.
- [x] Seamless single-sheet red metal front roll, subtle procedural powder-coat micro-grain, rounded S-curve stepped header cutouts (\ = 6\), and transparent-background gapped SVG group-box borders.
- [x] Original \
d matrix 1\ branding (D16) and full interactive controls + LED inventory across all sections.
- [x] All workspace Rust/Python quality gates pass unchanged.

## Out of Scope

- Porting the prototype into \crates/nord-panel\ / \crates/nord-gui\ or building the offline HTML-to-egui baker (follow-up task).
- Wiring live DSP audio output from \
ord-core\ into the HTML prototype.

## Summary of Changes

- Added \prototypes/panel-mock/index.html\, \prototypes/panel-mock/styles.css\, and \prototypes/panel-mock/panel.js\ implementing the interactive 1:1 \
d matrix 1\ HA88 front-panel prototype with section zoom inspector and full control interactivity.
