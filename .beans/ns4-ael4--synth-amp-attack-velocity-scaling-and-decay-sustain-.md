---
# ns4-ael4
title: Synth amp attack velocity scaling and decay sustain against the oracle
status: in-progress
type: task
created_at: 2026-11-21T20:00:00Z
updated_at: 2026-11-21T20:00:00Z
---

## Context
D200 (PR #571) left two oracle facts recorded but not implemented: the amp attack velocity scaling (`word * g / 127`, EMPIRICAL) and the decay sustain level 0.5.

## Higher Goal
Native synth envelope equals the ORG core 2 oracle (audio-remaining row 3).

## Acceptance Criteria
- [x] Amp attack law verified with the oracle on keys 36 / 60 / 84 and 9 key bytes; EMPIRICAL upgraded to VERIFIED with the corrected law `floor(w G / 128)`
- [x] Decay sustain verified (0.5000 on both chains), ARM write check
- [x] Opt-in `--synth-fw-adr-law` in native with per-program native vs oracle numbers
- [x] Tests, research note, DECISIONS D205, spec `synth-engine.md` 4.8
- [ ] CI green, PR merged

## Out of Scope
Default flips; the settled decay / release table; LFO and input gain into the cutoff (another session).

## Summary of Changes
See `docs/research/synth-env-law.md` and D205.
