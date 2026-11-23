---
# ns4-3o8r
title: Remove the vendor's product photos from the tree and the GUI binary
status: completed
type: task
priority: high
created_at: 2026-11-23T10:00:00Z
updated_at: 2026-11-23T12:00:00Z
---

## Context
`crates/nord-gui/assets/panel_texture.jpg` and `keyboard_full.jpg` are Nord's official product photographs of the Stage 4 (with the logo). `style.rs` embedded the first with `include_bytes!` (`PANEL_TEXTURE_BYTES`), so it shipped in every GUI binary. D16 says all shipped art is original and photos are never a source of pixels.

## Higher Goal
Keep the repository and its binaries free of vendor artwork (docs/LEGAL.md section 4).

## Acceptance Criteria
- [x] Both JPEGs removed from the tree; `PANEL_TEXTURE_BYTES` and the unused `load_panel_texture` removed (the GUI draws the baked plate, `panel/baked_view.rs`)
- [x] `tools/crop_panel.py` takes the photo as an argument and writes to `.reference/crops`; its tests use synthetic images
- [x] Stale "authentic Nord" comments in `style.rs` fixed
- [x] `tools/check_no_vendor_photos.py` (tests, CI step) fails if the photos return, a JPEG is tracked under `crates/` or `assets/`, or `include_bytes!` names a JPEG
- [x] D16, docs/LEGAL.md (sections 4 and 8) and the visual-style spec updated
- [x] Sweep of all other tracked images

## Out of Scope
Rewriting git history to purge the photos: owner decision (LEGAL.md section 8, question 5).

## Summary of Changes
Deleted both photos and the embedding code, reworked `crop_panel.py`, added the check with tests and a CI step, updated docs. Sweep result: all other tracked images are our own renders or GUI screenshots; `docs/img/gui/after-overview.png` shows an older "nord stage 4" header text, and `panel/mod.rs` still draws that text (not changed here).
