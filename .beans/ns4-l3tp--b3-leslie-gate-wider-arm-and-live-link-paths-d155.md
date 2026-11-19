---
# ns4-l3tp
title: 'B3 Leslie gate: wider ARM and live-link paths (D155)'
status: completed
type: task
created_at: 2026-11-17T20:16:01Z
updated_at: 2026-11-17T20:16:01Z
---

## Context
D148 (leslie-gate) left the FX cores, System routing ids, more programs, run-time loaded images and live chips untried for the install of 0x1256AD.

## Higher Goal
Native engine audio 1:1 with hardware through the ARM and SHARC oracles.

## Acceptance Criteria
- [x] Trace 7 programs, edit FX / Leslie / routing ids, scan every message for 0x1256AD / 0x1257B2
- [x] Live link on PNO 1 / 2 and ORG 1 / 2
- [x] docs/research/leslie-paths.md, DECISIONS D155, OPEN-QUESTIONS, ROADMAP
- [x] No native change (no measured gap)

## Out of Scope
Part command consumer, UART / MIDI gestures, SY1SY2 / SMP listings (follow-ups).

## Summary of Changes
tools/arm_leslie_paths.py + tests, docs/research/leslie-paths.md and data/leslie-paths.json, D155. Not found in the paths run.
