---
# ns4-sy8h
title: 'SY pitch entry e, eighth hunt: SY census rerun with the FIFO index bias'
status: completed
type: task
created_at: 2026-11-22T12:00:00Z
updated_at: 2026-11-22T12:00:00Z
---

## Context
D219 (seventh hunt, static census) ranked the SY FIFO bias as lead 2: D211 added the opt-in index bias for the SY1SY2 chip because the default model replays stale FIFO pairs, and left the SY census unrepeated. A message the ARM does send could have been lost or applied twice, hiding the writer of the 16-bit pitch entry `e` at `DM(0x9354A + index)`.

## Higher Goal
Find the writer of `e` so `--sample-pitch fw` can become the default (D22, D65, D89), or close each lead with evidence.

## Acceptance Criteria
- [x] Run the ARM against the live SY chip with and without `--fifo-pace 1 --index-bias 0x80000`, same program, with a watch on `DM(0x93500..0x9359F)`
- [x] Compare the two ARM to SY message scripts and the watch results with a tested tool (`tools/sy_script_window_census.py`)
- [x] Research note `docs/research/sy-fifo-bias-census-rerun.md`, D224, spec line, OQ22 and `audio-remaining.md` row 2 updated
- [x] No default, golden or `render_hash` moves

## Out of Scope
Flash directory model (lead 1), hardware clip (lead 3), interprocedural comm resolution (lead 4), SPI DMA opcode 9 (lead 5), a playing SY voice under the bias.

## Summary of Changes
- Result: negative. `SY0.script` is byte identical in both models (300 messages, 72 distinct addresses, 0 in the window); the 160 watched window words never change in 2,441 passes. The SY chip stayed idle (`lane_peak` 0.0), recorded as a limit.
- Added `tools/sy_script_window_census.py` and `tools/tests/test_sy_script_window_census.py`.
- Docs: `docs/research/sy-fifo-bias-census-rerun.md`, D224, `docs/specs/synth-engine.md`, `docs/OPEN-QUESTIONS.md` (OQ22), `docs/research/audio-remaining.md`.
