---
# ns4-pw7h
title: 'SY pitch entry e, seventh hunt: static census of ARM senders'
status: completed
type: task
created_at: 2026-11-22T09:00:00Z
updated_at: 2026-11-22T09:00:00Z
---

## Context
Six hunts (D105, D111, D112, D124, D141, D176, D196) found no writer of the SY pitch entry `e` at `DM(0x9354A + index)` by driving the ARM and the SY chip through harness paths. The remaining candidate was an ARM message with a computed address. This hunt asks statically which ARM code can send to the SY at all.

## Higher Goal
1:1 audio parity: `--sample-pitch fw` can only become the default once the firmware writer of `e` is known (D22, D65, D89).

## Acceptance Criteria
- [x] Static census of every call site of the path 1 writers with constant comm / word1 (`tools/arm_sender_sites.py`, `tools/arm_const_track.py`, tests)
- [x] Result recorded: 313 sites, 0 with a constant SY comm, 0 naming the window, SY table reach about `0x92E70`
- [x] D213 setter `0xC01C61F0` call graph followed (no edge to the window)
- [x] Research note, DECISIONS D219, spec paragraph, OQ22 and audio-remaining updated; ranked list of remaining leads
- [x] No default, golden or `render_hash` change; `--sample-pitch fw` stays opt-in

## Out of Scope
Flash file system model, SY FIFO bias fix in `sharc-live`, hardware clip check, interprocedural comm resolution (leads 1 to 4 of the note), any engine change.

## Summary of Changes
New `tools/arm_sender_sites.py` and shared `tools/arm_const_track.py` (the tracker moved out of `sy_flash_service_hunt.py`, behaviour kept), tests, note `docs/research/sy-pitch-writer-static-census.md`, D219, spec and open-question updates. Writer of `e` not found; negative evidence and ranked leads recorded.
