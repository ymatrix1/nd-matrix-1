---
# ns4-pdky
title: 'Piano: key-up and release placement under the sustain pedal (OQ20 residual)'
status: in-progress
type: task
priority: normal
created_at: 2026-11-28T15:00:00Z
updated_at: 2026-11-28T16:00:00Z
---

## Context
D262 (PR #699) made notes struck under the damper pedal take the firmware voice list. The firmware key-up record placement under the pedal (stroke kind 2) was left unmodelled (OQ20): the engine holds pedal-held voices and releases them at the lift with its own rule.

## Higher Goal
The release of a note struck or held under the sustain pedal matches the firmware: key-up record placed at the same moment, with the same velocity, gain and decay as the real ARM + SMP path.

## Acceptance Criteria
- [x] Firmware behaviour for cases (a) to (d) recorded with a reproducible command and derived numbers (`tools/arm_piano_keyup.py`, `docs/research/piano-pedal-keyup.md`)
- [x] Engine matches where the firmware is observable: plain and lift-first key-up, hold under the pedal already match (no change); re-strike under the pedal opt-in `--firmware-piano-pedal-restrike` with unit tests (D263)
- [x] Spec, decision entry, OQ20 and ROADMAP updated; all AGENTS.md checks pass; draft PR opened, not merged
- [ ] Pedal-lift key-up and release (cases (a), (b)) observed on the firmware: needs the harness to answer the SMP allocation reply so a second note-on starts after a key-up; then decode `0xC00AD4DC` and the timer job `0xC003EDE8`
- [ ] Half pedal law (CC64 64 half, 127 full, 65 to 126 up in the harness) mapped to the pedal jack path

## Out of Scope
Slew, chain scale and other piano flags; the Velvet clip spectral gap; anything outside pedal-related key-up placement.

## Summary of Changes
New `tools/arm_piano_keyup.py` (+ test), `piano/pedal_restrike.rs` (opt-in), research note, D263, spec, OQ20, ROADMAP, audit row.
