---
# ns4-ycc4
title: Firmware reverb port faster, bit-identical
status: in-progress
type: task
created_at: 2026-11-19T04:00:09Z
updated_at: 2026-11-19T04:00:09Z
---

## Context
The GUI Reverb checkbox (firmware reverb port) worsens crackle; the port costs about 1 ms per rack and 480 frame block.

## Higher Goal
GUI audio without block overruns with firmware FX (docs/research/gui-audio-glitches.md, TD-14).

## Acceptance Criteria
- [x] Output bit-identical (render_hash, goldens, pinned type/colour hashes)
- [x] Per type before/after table and profile in docs/research/fw-reverb-speed.md
- [ ] 2x on the reverb port (reached about 1.3x; LTO would add 1.15 to 1.2x)

## Out of Scope
Rack scheduling, idle skip, audio thread (gui-crackle session); release profile change (owner decision).

## Summary of Changes
E40 mul/add fast paths and inlining, paired window access, tests_fw_reverb_pinned.rs, fx_reverb_speed bench.
