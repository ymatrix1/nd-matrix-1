---
# ns4-pt0l
title: 'Plugin: top bar pills load firmware, .ns4b and programs'
status: in-progress
type: task
created_at: 2026-11-22T10:00:00Z
updated_at: 2026-11-22T10:00:00Z
parent: ns4-xc5n
---

## Context
The three top-bar pills of the plugin editor (`OPEN .NS4B`, `DEBUG`, `? HELP`) drew but did nothing, and the plugin had no way to load firmware, a `.ns4b`, a `.ns4p` or a sample. The engine lives on the audio thread, `process()` may not allocate or lock, and the block sync overwrote a loaded program from the host parameters.

## Higher Goal
Clicking OPEN .NS4B in a DAW loads an `.ns4b` and a program that then sounds, like the standalone app (D218, `docs/specs/daw-plugin.md` Loading).

## Acceptance Criteria
- [x] OPEN .NS4B opens the Library tab, DEBUG the diagnostics dialog, ? HELP the Setup Guide
- [x] Bounded queue GUI thread to audio thread for Program, Piano, Sample and Firmware jobs; zero allocator calls in `process()` (counting allocator tests)
- [x] Replaced values freed on the reaper bin, not on the audio thread
- [x] A loaded program survives the block sync per group until the host moves that group (`program_hold.rs`)
- [x] File picker on a worker thread (`rfd`, feature `picker`)
- [x] Headless end-to-end test: OPEN pill, synthetic `.ns4b`, program, audible `process()` (`tests/load_path.rs`)
- [x] D218, `docs/specs/daw-plugin.md`, TD-21, `docs/plugin-editor.md`, ROADMAP B11a updated
- [x] Load checked by hand in FL Studio (2026-11-22, `HOST_COMPATIBILITY.md` 7.9)
- [ ] Load checked by hand in Reaper and Bitwig

## Out of Scope
- Pushing program values into host parameters; persisting the loaded program in project state; `LoadPreset` (TD-21)
- A DAW run (`docs/HOST_COMPATIBILITY.md` 7.7)

## Summary of Changes
New `loader.rs`, `program_hold.rs`, `core_sync.rs`, `wave_tables.rs` in `crates/nord-plugin`; editor dialogs and top bar wiring; `nord_gui::file_picker` worker-thread picker; embedded debug dialog.
