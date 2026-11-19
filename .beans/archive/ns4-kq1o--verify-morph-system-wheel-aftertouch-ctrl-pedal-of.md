---
# ns4-kq1o
title: Verify morph system (wheel, aftertouch, ctrl pedal) of native vs firmware
status: completed
type: task
created_at: 2026-11-17T23:44:09Z
updated_at: 2026-11-17T23:44:09Z
parent: ns4-trkb
---

## Context
Owner: A:16 Horizon Whl sounds very different from the hardware video; layer A has a wheel morph on cutoff (0.079), 3 synth layers. D81 clip comparison never included wheel/arp.

## Higher Goal
Audio 1:1 with hardware from ARM+SHARC firmware execution only.

## Acceptance Criteria
- [x] Per morph target: native law vs firmware law in numbers (value to word to Hz/dB)
- [x] docs/research/morph-fw-vs-native.md + data under docs/research/data/
- [x] Fixes only behind opt-in flags; DECISIONS entry; ROADMAP/OQ sync
- [x] Tests, checks, PR

## Out of Scope
Default flips (owner decides). Arpeggiator.

## Summary of Changes
- ARM harness `tools/arm_program_morph.py` (recall a program, drive CC1/CC11/D0, log dispatch and DSP words), fit report `tools/morph_law_report.py`, test-program writer `tools/morph_test_program.py`, with tests.
- Firmware law: weight `v / 128`, clamp on the sum, cutoff sender drops a step from frac 0.5 (245 series; `docs/research/morph-fw-vs-native.md`, data `docs/research/data/morph-*.csv`).
- Native: opt-in `--firmware-morph` (`MorphControllers::firmware_weighted`, `synth_morph.rs`, sender step drop); default unchanged.
- D159 (proposed), ROADMAP and OQ30 / OQ31 synced.
- Horizon Whl: nothing differs at wheel 0; with wheel up native lacks the synth B env-amount morph and is up to 0.5 dB off on cutoff. Morph does not explain the owner's gap.
