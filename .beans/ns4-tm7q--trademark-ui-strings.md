---
# ns4-tm7q
title: 'Trademark cleanup: ND Matrix 1 name in user-facing strings'
status: completed
type: task
created_at: 2026-11-23T09:00:00Z
updated_at: 2026-11-23T09:00:00Z
---

## Context
D16 renamed the product to "ND Matrix 1" so the Nord trademark is not our product name. Some GUI strings still used "Nord Stage 4" as a title or brand (library drawer heading, file filter, window class, default program title, chassis wordmark).

## Higher Goal
Ship a product whose own name never uses the Nord trademark; "Nord" appears only descriptively.

## Acceptance Criteria
- [x] Sweep user-visible strings in `nord-gui`, `nord-plugin`, `nord-panel`, `ns4` help, release tools and `docs/RELEASE_README.txt`; classify every hit
- [x] Brand-like strings renamed to "ND Matrix 1" or a descriptive phrase
- [x] Host parameter IDs, CLAP/VST3 ids, crate names and hardware data matches unchanged
- [x] Unit tests pin each changed string (library title, file picker filter, default title, wordmark, window class)
- [x] Naming rule recorded in `docs/specs/visual-style-and-panel.md` and linked from D16
- [x] `python tools/check_tests.py` stays at 0

## Out of Scope
- Renaming crates, binaries or the repository
- Changing plugin ids, host parameter ids or `prog.name == "Nord Stage 4 Program"` data matching
- Replacing the baked panel art itself (tracked as an open question in the PR)

## Summary of Changes
- New `crates/nord-gui/src/branding.rs` holds the brand-like strings with tests.
- Library heading, file-dialog filter, window class, default program title, chassis wordmark and two setup-dialog texts now use ND Matrix 1 or descriptive wording.
- Spec section 10 in `visual-style-and-panel.md` and a D16 follow-up line.
