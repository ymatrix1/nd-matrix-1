---
# ns4-8zdf
title: 'GUI crackle round 2: audio thread priority, allocation-free program load, clamp and gap meters'
status: completed
type: task
created_at: 2026-11-19T03:21:57Z
updated_at: 2026-11-19T03:21:57Z
parent: ns4-trkb
---

## Context
The owner still hears crackle in nord-gui after PR 517 (D161). cpal 0.15.3 never raises the WASAPI callback thread (it passes the thread id as a HANDLE), so the callback runs at normal priority and loses blocks to other processes. Program load and Mod 2 type changes build stage boxes and free program boxes on the audio thread (TD-18, TD-12).

## Higher Goal
Glitch-free GUI audio with the default settings.

## Acceptance Criteria
- [x] Real cpal run reproduces the late blocks and shows the fix (nord-stage4 --audio-probe)
- [x] Audio thread priority raised (MMCSS Pro Audio + TIME_CRITICAL) with a test
- [x] Program load and Mod 1 / Mod 2 type changes make 0 allocator calls on the audio thread (counting allocator test)
- [x] Meter reports callback gaps, master clamp hits and the thread class
- [x] ns4 bench --gui --reload scenario, docs (TD-12, TD-14, TD-18, gui-audio, research), D167
- [x] fmt, clippy, cargo test, ruff, pytest, check_tests pass; render_hash unchanged

## Out of Scope
Default flips that change sound (idle skip, soft limiter), reverb speedups, panel files.

## Summary of Changes
- Finding: cpal 0.15.3 passes the thread id as a `HANDLE`, so its priority boost fails; the callback ran at priority 0 without MMCSS. Under CPU contention the unboosted callback lost 13 blocks (worst 20.9 ms) on a 26 percent load render; boosted: 0 late, worst 4.5 ms (`nord-stage4 --audio-probe`).
- `priority.rs` (TIME_CRITICAL + MMCSS `Pro Audio`, `--no-thread-boost`), meter gap / clamp / thread class, `MasterBus::clamped_frames` (default start-up engine in Scene II clamps 6 to 49 percent of the frames).
- `garbage.rs` reaper queue for spent program boxes and replaced sound banks; `reserve_firmware_stages` and in-place re-init of Mod 1 / Mod 2 stage boxes; counting-allocator test shows 0 allocator calls on program loads.
- `ns4 bench --gui --reload / --reload-pool / --program2`, `nord-stage4 --audio-probe`; D167, TD-12 / TD-14 / TD-18, gui-audio spec, research note section 6, ROADMAP.
