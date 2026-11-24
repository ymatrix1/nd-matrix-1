---
# ns4-jfym
title: 'Synth filter stages vs Rust model: per-stage oracle diff (D122)'
status: completed
type: task
priority: normal
created_at: 2026-11-17T02:27:02Z
updated_at: 2026-11-17T02:27:25Z
parent: ns4-trkb
---

## Context
D121 left the fitted per-type/row cutoff factors as the gap of `--synth-fw-filter` (3.36 vs 2.18). Hypothesis: they compensate a port error in the Rust stage model.

## Higher Goal
Audio 1:1 with hardware via the SHARC oracle.

## Acceptance Criteria
- [x] Each real stage (0x1253BD, 0x12566E, 0x12549D, 0x125913, 0x125A50) driven in the oracle with voice frame words and diffed per sample against FwCoefs/FwFilter
- [x] Response (corner, peak, slope) per stage, type, resonance, cutoff against the voice chain
- [x] Verdict, research note, spec/OQ22/ROADMAP, D122

## Out of Scope
Stage 0x1257B2, F vs 2F, default changes, JIT/AOT.

## Summary of Changes

- `crates/nord-tools/src/bin/fw_filter_stage_dump.rs` (Rust stage dumper), `FwCoefs::new_strip` (tool-facing).
- `tools/synth_filter_stage_diff.py` (+ test): `words`, `diff`, `corner`, `ladder-voice`.
- Verdict: LP12/LP24/BP/HP/Ladder24 equal the oracle to 3.3e-5 .. 1.4e-4 of the output peak; per-type factors are not a port error; Ladder24 position word in a sounding voice is open. `docs/research/synth-filter-stage-vs-model.md`, D122, OQ22, ROADMAP, spec s4.6.
