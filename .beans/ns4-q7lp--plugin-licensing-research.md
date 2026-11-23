---
# ns4-q7lp
title: Research DAW plugin licensing (VST3, CLAP, LV2) and propose D233
status: completed
type: task
priority: normal
created_at: 2026-11-23T09:00:00Z
updated_at: 2026-11-23T10:00:00Z
---

## Context
`crates/nord-plugin` depends on nih-plug with `vst3` and `standalone` features. The VST3 binding `vst3-sys` is GPLv3, which conflicts with a PolyForm Noncommercial core when the plugin binary is distributed. Archived bean `ns4-815y` planned a GPL plugin license but no license file is on `main`.

## Higher Goal
Release a DAW plugin without silently granting GPLv3 (and commercial) rights over the core.

## Acceptance Criteria
- [x] Identify the VST3 binding crate and every non-permissive license in the plugin dependency tree (`cargo metadata`, `cargo tree`)
- [x] Check the Steinberg MIT relicense, permissive VST3 binding options and nih-plug status against primary sources
- [x] Check CLAP, `standalone` and LV2 licenses and options
- [x] `docs/research/plugin-licensing.md` in the research format
- [x] Decision D233 (proposed) with options a to e and a recommendation
- [x] Open question OQ45 added
- [ ] Owner decision on D233 (out of this bean)

## Out of Scope
- Any code, Cargo feature or LICENSE file change
- Legal advice

## Summary of Changes
- Added `docs/research/plugin-licensing.md` (dependency license table, VST3/CLAP/LV2/standalone findings, unverified items).
- Added D233 (proposed) to `docs/DECISIONS.md`, recommending CLAP and standalone only until a permissive VST3 binding is validated.
- Added OQ45 to `docs/OPEN-QUESTIONS.md`.
