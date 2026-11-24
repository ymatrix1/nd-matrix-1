---
# ns4-6da7
title: Rank all 384 programs by native vs firmware-chain distance
status: completed
type: task
priority: normal
created_at: 2026-11-17T23:18:57Z
updated_at: 2026-11-18T00:52:43Z
parent: ns4-trkb
---


## Context
Native engine vs firmware chain (ARM harness + SHARC oracle live link) is checked on a handful of programs. No table says where native is farthest across the 384 factory programs.

## Higher Goal
Audio 1:1 with hardware from ARM + SHARC firmware execution only. Point other work at the worst gaps.

## Acceptance Criteria
- [x] Deterministic numbers-only table of 384 programs under docs/research/data/
- [x] docs/research/program-ranking.md: worst 20, grouping by engine and cause
- [x] Tests for new code, files < 400 lines, provenance tags
- [x] DECISIONS entry + index row, ROADMAP/OQ sync
- [x] All AGENTS.md checks pass, PR opened

## Out of Scope
Default flips, native DSP changes, committing firmware/sample/audio content.

## Summary of Changes
- `crates/nord-tools`: `sweep_load.rs` (loading code moved out of `program_sweep`), `rank_metrics.rs`, bin `program_rank` (`--ablate`), tests.
- `tools/program_rank_chain.py` (live-link census of all 384), `tools/program_rank.py` (score, flags, tables), tests.
- Data `program-ranking.csv`, `program-ranking-ablation.csv`; note `program-ranking.md`; D160, OQ40, ROADMAP 7b.
- Finding: all of the worst 20 are FX (Mod 2 12); the chain has no piano tap; 13 synth and 5 organ programs are silent in the chain at key 60 (OQ40).
