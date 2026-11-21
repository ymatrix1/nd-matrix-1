---
# ns4-cu7t
title: Synth cutoff index sum: LFO and input gain terms, the 80s Sci-Fi base offset (D204)
status: completed
type: task
created_at: 2026-11-21T18:00:00Z
updated_at: 2026-11-21T21:00:00Z
---

## Context
D200 (PR 571) measured how the filter envelope adds into the strip cutoff `R` and left the LFO and input gain terms unmeasured, plus 80s Sci-Fi with a base `R` of 14.7 kHz against 1.5 kHz of the law.

## Higher Goal
Audio-remaining row 3: the synth gap outside the filter stages. The firmware cutoff composition must be complete before any default flip.

## Acceptance Criteria
- [x] Measure the LFO and input terms of the cutoff index sum with the ORG core 2 oracle (`tools/synth_cutoff_terms.py`, 40 rows).
- [x] Explain the 80s Sci-Fi base offset (Ladder24 reads the linear ramp table).
- [x] Port the verified terms into `synth/fw_cutoff_env.rs` behind `--synth-fw-env-law` with tests (no default flip).
- [x] Per-program native against oracle CSV (`synth-env-trace-index.csv`).
- [x] D204, `docs/specs/synth-engine.md` 4.7, audio-remaining row 3, ROADMAP.

## Out of Scope
Default flip, amp attack velocity scaling, a running LFO (rate words of PNO core 1), layers B / C strip injection.

## Summary of Changes
- Index sum `idx = w3 + m0 + ((w5 + m1) lfo << 2) + 2 w7 FIX(F-4)`: LFO `512 depth lfo`, key offset 1:1, envelope `128 w7 F-4`, no input gain term (drive byte does not move `R`); ARM LFO depth word `kLFOAmt / 127 * 0x40000000` (`tools/synth_lfo_words.py`).
- 80s Sci-Fi: stage `0x12549D` reads the ramp table, others the exponential law; `fw_stage_r_of_knob`.
- Code: `fw_cutoff_env.rs` (`fw_lfo_semitones`, `fw_lfo_cutoff_hz`, `fw_stage_r_of_knob`), `render.rs` LFO wiring, `synth_env_native --note-offset --kb-track`, `synth_env_trace.py` index report. Defaults unchanged.
- Docs: `synth-env-oracle-trace.md` s7, D204, spec 4.7, audio-remaining row 3, ROADMAP.
