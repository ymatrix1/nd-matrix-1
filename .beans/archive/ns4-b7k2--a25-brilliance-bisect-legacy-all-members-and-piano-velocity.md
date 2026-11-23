---
# ns4-b7k2
title: 'A:25 brilliance bisect: legacy_all members and piano velocity (D221)'
status: completed
type: task
created_at: 2026-11-22T12:00:00Z
updated_at: 2026-11-22T18:00:00Z
---

## Context
D215 (PR 589) found the common treble excess gone on `main` except A:25, still +14.8 dB at 3..8 kHz; 8 dB of it sat in a non-FX default that was not bisected (the members of `FirmwareOptions::legacy_all`). Piano velocity also moves piano_dry brilliance.

## Higher Goal
1:1 audio parity with the hardware: name the stage or default that makes A:25 too bright and put the fix behind an opt-in flag (the owner decides flips).

## Acceptance Criteria
- [x] Per-member band deltas of `legacy_all` on A:25 (31 variants, forward and reverse) in `docs/research/a25-brilliance-bisect.md`
- [x] Responsible default identified: `SynthChain::FirmwareStrip`, inside it `SUPER_LP24_TYPE_SCALE = 4.4`
- [x] Fix behind an opt-in only: the existing `--synth-super-lp24`; no default flips
- [x] Piano velocity of the hardware clip estimated (80 to 85, EMPIRICAL)
- [x] Provenance tags, unit tests (`tests_super_lp24_a25.rs`), DECISIONS D221, audio-remaining row 23, effects and synth specs updated
- [x] AGENTS.md checks pass, PR opened

## Out of Scope
- Flipping `SUPER_LP24_TYPE_SCALE` (D116 criteria open); reading the wheel position; the `synth.layer_a.enabled` edit having no effect.

## Summary of Changes
Bisect found `--synth-chain legacy` is the only `legacy_all` member that moves A:25 (+14.2 to +6.1 dB); the clip is not piano only, its SuperSaw LP24 synth layer carries the 8 dB through the 4.4 type factor. `--synth-super-lp24 2.0` gives +5.8. Research note, D221, spec and row 23 updates, spec JSONs and a layer-level test.
