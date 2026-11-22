---
# ns4-ev12
title: 'Synth envelope: settled decay / release tables, amp chain limit above 1, per program error'
status: completed
type: task
created_at: 2026-11-21T23:00:00Z
updated_at: 2026-11-21T23:30:00Z
---

## Context
D210 found the default synth attack table is a stale 3 pass sweep (before the installer's 16 pass refresh) and left the decay / release table, the amp chain peak above 1 at `g >= 107` and the per program cost open.

## Higher Goal
Give the native synth envelope chain oracle exact rate tables and decide the default flip with evidence (D22: owner approves).

## Acceptance Criteria
- [x] Decay and release tables re-measured at all 129 indexes after the 16 pass settle; provenance tags; Rust tables with a CSV equality test
- [x] Amp chain peak above 1 explained or bounded (fitted law, 592 of 592 oracle rows; 0 of 635 factory layers reach it)
- [x] Per program native vs oracle error for stale vs settled tables (40 layers) and the envelope index census of the 384 programs
- [x] Default flip proposed, not applied (`DEFAULT_TABLES`)
- [x] Tests, research note, DECISIONS D212, spec `synth-engine.md` 4.8, ROADMAP

## Out of Scope
The default flip, a clip score on the hardware recordings, porting the amp limit law, inverting the settled table for the panel time path.

## Summary of Changes
`synth/fw_env_settled.rs`, `RateTables` in `fw_env_rates.rs`, `strip_path.rs`; `synth_env_census` and `synth_env_native --rates --sustain-half`; `tools/synth_env_rates.py`, `synth_env_limit.py`, `synth_env_programs.py` with tests; data CSVs; `docs/research/synth-env-rates-settled.md`; D212.
