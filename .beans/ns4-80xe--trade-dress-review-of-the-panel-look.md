---
# ns4-80xe
title: 'Proactive trade dress mitigation: neutral non-red default panel theme'
status: completed
type: task
priority: high
created_at: 2026-11-23T16:00:00Z
updated_at: 2026-11-23T21:25:00Z
parent: ns4-9wq1
---

## Context
All shipped art is original (D16; vendor photos removed and blocked by `tools/check_no_vendor_photos.py`; the Blender proof `assets/blender/build_proof.py` builds geometry from primitives with procedural materials). However, the panel still reproduces the hardware's signature red chassis colour and section layout closely (`crates/nord-gui/assets/baked/panel_plate_2x.png`). Trade dress (overall look and feel) is a separate risk from copyright and trademarks, and under a $0 legal budget the safest move is proactive visual differentiation rather than paying counsel to defend a red panel.

## Higher Goal
The default GUI look and all public screenshots have a distinct, non-red ND Matrix 1 visual identity that does not mimic the vendor's signature red hardware chassis.

## Acceptance Criteria
- [x] Design and bake a neutral, non-red default chassis theme (e.g., dark charcoal/slate/navy) with clear ND Matrix 1 branding
- [x] Owner request 2026-11-23: change colors, images and other visual elements (knob and button shapes, key colors, section accents, typography, plate texture) so the GUI looks clearly different from the original Nord panel, not only a recolor of the chassis
- [x] Make the neutral theme the default in `nord-gui` and `nord-plugin`
- [x] Refresh README and release screenshots (`docs/img/gui/`) to show the neutral default theme
- [x] Decision recorded in `docs/DECISIONS.md` and `docs/specs/visual-style-and-panel.md`

## Out of Scope
Removing the panel GUI. Moving controls or changing parameter IDs (D16).

## Summary of Changes
- New original "Slate Matrix" identity (D246): graphite-navy dot-matrix chassis, chamfered dark headers with per-section accents, bone sub-panels, hexagonal pot knobs, teal-ringed encoders, tile buttons, mint/cyan LEDs, ivory and blue-black keys, monospace silkscreen, `ND MATRIX 1` lockup. Replaces the red look in `prototypes/panel-mock/`; re-baked with `tools/bake_panel_from_html.py` (deterministic: a second bake gave identical hashes).
- One default for `nord-gui` and `nord-plugin`; the old red theme, red screenshots and red Blender proof plate are deleted, not hidden behind a switch.
- Ids renamed off the vendor colour: `LED_RED` to `LED_ACCENT`, `CHASSIS_RED` to `CHASSIS_BASE`, `PROGRAM_RED` to `PROGRAM_PLATE`, `btn_red_*` to `btn_accent_*`, variant `red` to `accent`.
- Guards: `crates/nord-gui/src/identity_tests.rs`, `tools/tests/test_bake_panel_from_html.py`.
- Docs: D246, `visual-style-and-panel.md` s1 s2 s9, ROADMAP, LEGAL follow-ups, `docs/img/gui/overview.png`, README.
