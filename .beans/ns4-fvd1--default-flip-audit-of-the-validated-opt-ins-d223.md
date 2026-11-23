---
# ns4-fvd1
title: 'Default flip audit of the validated opt-ins (D223)'
status: completed
type: task
priority: normal
created_at: 2026-11-22T12:00:00Z
updated_at: 2026-11-22T12:00:00Z
---

## Context
The owner said "flip what was validated" (2026-11-22). Candidates: `--synth-fw-env-law`, the settled envelope tables, `--synth-fw-adr-law`, `--synth-osc-env-morph`, `--firmware-piano-timbre`, `--fx-compressor-first`, the `lp24` FX port, `--synth-fw-resonance`, `--organ-level chain`. D22 and the owner rule of 2026-11-17: oracle VERIFIED, no ASSUMED / EMPIRICAL link left, no hardware clip worse.

## Higher Goal
Audio 1:1 with the hardware from firmware and oracle evidence only; the default is the closest verified path.

## Acceptance Criteria
- [x] Per-candidate verdict (flip, or keep opt-in with the reason) in D223 and [default-flip-audit](../docs/research/default-flip-audit.md)
- [x] Flip with a `--legacy-*` escape: `--synth-fw-resonance` (`--legacy-synth-resonance`, also in `program_sweep`)
- [x] Hardware clip A/B (17 synth segments) and the 384 program sweep before and after
- [x] Golden `render_hash` re-baselined (`engine`, `synth`), numbers in D223
- [x] Spec `synth-engine.md` s4.6, ROADMAP, audio-remaining, firmware-flag-audit
- [x] All AGENTS.md checks

## Out of Scope
Reverb ports, Mod 2, unison / per-type gain, `--synth-filter-gain`, the D203 FX words; the ASSUMED links of the envelope law and the settled tables (follow-ups in the research note).

## Summary of Changes
- `--synth-fw-resonance` is the default; the envelope law, settled tables, adr law, osc env morph, piano timbre, compressor first, lp24 and organ chain stay opt-in, each with its reason and the evidence needed ("needs owner / hardware" list in D223).
