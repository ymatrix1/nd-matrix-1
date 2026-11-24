---
# ns4-b3ld
title: 'B3 Leslie darkness (-12.8 dB at 3..8 kHz) is the replayed note set, not a stage (row 9)'
status: completed
type: task
created_at: 2026-11-22T00:00:00Z
updated_at: 2026-11-22T00:00:00Z
---

## Context
audio-remaining row 9 and D215 read the B3 clip 12.8 dB below the hardware in 3..8 kHz and blamed the Leslie.

## Higher Goal
1:1 audio parity: find the responsible organ stage or show there is none, without fitting a stage to a wrong measurement.

## Acceptance Criteria
- [x] Bisect the chain (rotary on / off, drive scale, firmware Leslie, firmware options) against the clip with `tools/emu_vs_hw.py`
- [x] Identify the cause with evidence: note-set dependence, 24 dB across key zones (`docs/research/b3-leslie-dark.md`)
- [x] Opt-in tool fix: `emu_vs_hw.py --merge-gap` with a unit test; no default flips
- [x] Research note, DECISIONS D220, audio-remaining rows 9 and 23, emulator-vs-hardware-audio updated
- [x] Checks from AGENTS.md pass

## Out of Scope
Engine changes, a firmware Leslie port or install (D190, OQ42), a second B3 recording, flipping `B3Wheels` (D22; follow-up 1 of the note).

## Summary of Changes
- `tools/emu_events.py`, `tools/emu_vs_hw.py`: `--merge-gap SECONDS` (default off); test in `tools/tests/test_emu_events.py`.
- `docs/research/b3-leslie-dark.md`, data `emu_vs_hw_b3_leslie_dark{,_results}.json`, D220.
- B3 brilliance on the merged stream: +1.3 dB, presence -3.0 dB.
