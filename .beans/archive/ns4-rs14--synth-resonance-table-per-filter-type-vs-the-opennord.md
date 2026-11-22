---
# ns4-rs14
title: 'Synth resonance table per filter type vs the OpenNord quadratic (D214)'
status: completed
type: task
created_at: 2026-11-21T23:50:00Z
updated_at: 2026-11-21T23:50:00Z
---

## Context

D212 left open why the resonance damping table `B` (`0xB1015`) equals the OpenNord quadratic `k = 2 (1 - v / 128)^2` only below index 38. Which side is right for the firmware, per filter type?

## Higher Goal

Native synth filter resonance at parity with the ORG core 2 stages.

## Acceptance Criteria

- [x] Oracle measurement of the stage damping and input gain against the position word, all positions, LP12, LP24, BP, HP; Ladder24 coefficient words against the model
- [x] All 128 knobs settled, compared with the native table lookup and position map
- [x] Research note, DECISIONS entry (D214), spec update
- [x] Native fix behind an opt-in flag only if the table differs (it does not; test added instead)
- [x] No default flips (proposal only)

## Out of Scope

Flipping `--synth-fw-resonance` to default; porting the 0.995 per pass position smoother; hardware capture of the audible Q.

## Summary of Changes

The quadratic is the LP12 table `D`, not `B`; both native tables and the per-kind selection equal the oracle (1e-7); new tool `tools/synth_res_oracle.py`, derived CSVs, a Rust test over all 128 knobs, `docs/research/synth-resonance-law.md`, D214, `specs/synth-engine.md`.
