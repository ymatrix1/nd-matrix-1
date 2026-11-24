---
# ns4-tmpm
title: ARM to DSP message capture over all 384 factory programs, sample-pitch e check
status: completed
type: task
priority: normal
created_at: 2026-11-17T02:00:07Z
updated_at: 2026-11-17T03:11:41Z
parent: ns4-trkb
---


## Context
SY sample pitch entry e (DM(0x9354A+voice)) resisted four hunts (D89, D105, D111, D112). Hypothesis: it arrives from the ARM message stream or SPI flash DMA.

## Higher Goal
Audio 1:1 with hardware; close OQ22.

## Acceptance Criteria
- [x] e verdict for the SAMPLE programs, with the evidence command
- [x] Capture of all 384 programs, summary table by engine
- [x] tools/arm_dsp_capture.py, tools/arm_dsp_summary.py with tests
- [x] docs/research/arm-dsp-capture.md, decision D124

## Out of Scope
SHARC code changes, JIT/AOT, SPI flash model, flipping defaults.

## Summary of Changes
Added `tools/arm_dsp_capture.py` (resumable batch capture, 384 of 384 programs, 0 failed) and `tools/arm_dsp_summary.py` with tests, the research note `docs/research/arm-dsp-capture.md`, small summaries under `docs/research/data/arm-dsp-capture-*`, and D124. Verdict: no ARM word in `DM(0x93500..0x935A0)` in 119 of 119 SAMPLE programs; no sample id, address or descriptor sent; recall sends 0 words to SY. OQ22, audio-remaining, emulation-coverage, ROADMAP and the synth-engine spec updated.
