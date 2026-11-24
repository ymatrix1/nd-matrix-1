---
# ns4-oq44
title: 'OQ44: Env To Pitch DSP scale and native consumption'
status: completed
type: task
created_at: 2026-11-21T00:00:00Z
updated_at: 2026-11-21T00:00:00Z
---

## Context
OQ44: DSP consumer of Osc Env Amount word 12 under Env To Pitch.

## Higher Goal
1:1 synth pitch parity behind --synth-osc-env-morph.

## Acceptance Criteria
- [x] Oracle scale, curve, sign: 48 * word * level semitones (46 rows, 0.03 st)
- [x] Native consumes osc_env_dsp_word behind the flag, decay stops at 0.5
- [x] Oracle vs native trace on affected programs (8 traced, 1 decisive, 2 unresolved)
- [x] Tests, research, D217, spec, OQ44 closed

## Out of Scope
Default flip, layers B/C, velocity term.

## Summary of Changes
See docs/research/synth-osc-env-pitch.md and D217.
