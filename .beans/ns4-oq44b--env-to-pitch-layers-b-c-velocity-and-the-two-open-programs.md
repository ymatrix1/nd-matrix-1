---
# ns4-oq44b
title: 'Env To Pitch: layers B / C, Circus Box, Cinema Rise, velocity term (OQ44 follow-up)'
status: completed
type: task
created_at: 2026-11-22T00:00:00Z
updated_at: 2026-11-22T00:00:00Z
---

## Context
D217 verified `48 * word * level` semitones on layer A of 8 programs. Layers B and C, the Circus Box and Cinema Rise traces and the key velocity term were open.

## Higher Goal
1:1 synth pitch parity behind `--synth-osc-env-morph`.

## Acceptance Criteria
- [x] Oracle layer addressing for B and C (slot = layer: strip 2 L, common block 32 L, chain `F + 6`, voice record 54 L)
- [x] Held-level law of all 13 layers with an amount (`tools/synth_osc_env_pitch.py layers`)
- [x] Circus Box and Cinema Rise A / B / C resolved against the oracle
- [x] Key velocity term measured and implemented behind the flag (`G / 128`)
- [x] Tests, research note, D219, spec

## Out of Scope
Default flip; `kOscEnvDst = 0` depth; audible A/B; why Bounce Clav B and Lyle Lead J3 B are silent in the oracle voice record.

## Summary of Changes
See docs/research/synth-osc-env-pitch.md and D219.
