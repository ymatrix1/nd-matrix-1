---
# ns4-pvb3
title: 'Replace GPLv3 vst3-sys with the permissive vst3 crate in the DAW plugin (D233)'
status: in-progress
type: task
priority: normal
created_at: 2026-11-23T13:00:00Z
updated_at: 2026-11-23T13:00:00Z
---

## Context
The project license is PolyForm Noncommercial 1.0.0 and no shipped binary may permit commercial use. `nord-plugin` used nih-plug `de421011` with the `vst3` feature, which pulls `vst3-sys` 0.1.0 (GPLv3), the only copyleft crate in the plugin tree. The owner chose to keep VST3 and move to the MIT or Apache-2.0 `vst3` crate (coupler-rs, 0.3.0).

## Higher Goal
Ship CLAP, VST3 and standalone binaries that can all be distributed under PolyForm Noncommercial 1.0.0, without breaking saved DAW projects (D16).

## Acceptance Criteria
- [x] nih-plug variant with a `vst3`-crate wrapper chosen, pinned by `rev`, and the choice and licenses recorded (`docs/research/permissive-vst3-binding.md`)
- [x] `crates/nord-plugin`, `crates/xtask`, `vendor/nih_plug_egui` and the workspace `[patch]` use it; CLAP and standalone still build
- [x] `cargo tree -p nord-plugin -i vst3-sys` finds nothing
- [x] CLAP ID, VST3 class ID and parameter-ID hashes pinned by tests; `param-ids.snapshot` unchanged
- [x] `tools/check_licenses.py` with tests, wired into CI as job `licenses`
- [x] Plugin crate tests, bundle build, `clap-validator` and the Steinberg `validator` run (7.10)
- [x] HOST_COMPATIBILITY 7.10, RELEASING checklist and `docs/specs/daw-plugin.md` updated
- [ ] Owner runs the manual VST3 host checklist (HOST_COMPATIBILITY 7.10)
- [ ] D233 in `docs/DECISIONS.md` set to option (b), accepted, after PR #614 merges

## Out of Scope
LV2 export, AUv2, moving the egui version, nice-plug migration, any change to plugin behaviour, parameters or state format, legal review of the Steinberg SDK headers behind the `vst3` bindings.

## Summary of Changes
Pinned nih-plug PR #263 head `95034d5` (fork `davemollen/nih-plug`), removing `vst3-sys` and the three `vst3-com*` crates and adding `vst3` 0.3.0 and `com-scrape-types` 0.1.1 (both `MIT OR Apache-2.0`). Added `plugin_ids` tests, `tools/check_licenses.py` (500 crates checked, fails on the old tree with `vst3-sys 0.1.0`), the CI `licenses` job, a research note and the doc updates. Host testing and the D233 text change are pending.
