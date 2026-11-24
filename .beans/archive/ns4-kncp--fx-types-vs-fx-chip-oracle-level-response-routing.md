---
# ns4-kncp
title: 'FX types vs FX chip oracle: level, response, routing per type'
status: completed
type: task
created_at: 2026-11-17T23:10:43Z
updated_at: 2026-11-17T23:10:43Z
parent: ns4-trkb
---

## Context
Only reverb type 0 and chorus were compared against the firmware FX chip run in the SHARC oracle (D125, D133). The other native FX types have no live-chain comparison.

## Higher Goal
Audio 1:1 with hardware using only ARM+SHARC firmware execution.

## Acceptance Criteria
- [x] Deterministic stimuli (impulse, sweep, noise, held-note stream) rendered by native engine and firmware FX chip (oracle) per FX type and parameter step
- [x] docs/research/fx-vs-oracle.md with a table per type: match / differs by X dB / not runnable
- [x] Data under docs/research/data/, tests, provenance tags, DECISIONS entry, ROADMAP/OQ sync
- [x] Fixes only behind opt-in flags

## Out of Scope
Default flips; Leslie gate install (D148/D155, record only).

## Summary of Changes
- `fx/fw_live.rs`: `live_install` exports port contexts (Mod 1, Mod 2, compressor) for the oracle; `tools/fx_vs_oracle.py` runs oracle and port on the same stimuli.
- Opt-in `LayerFxChain::mod1_rest_frames` (default 2, 4 matches the live pass: null -100 dB).
- `fx_engine_vs_fw` + `fx_cases.rs`: engine against port for every family; data in `docs/research/data/fx-*.csv`.
- `docs/research/fx-vs-oracle.md`, D158, OQ38, ROADMAP 7b.4.
- Result: every runnable port matches the oracle; engine paths differ (up to 17 dB reverb, 5.9 dB Mod 2). Not runnable: Leslie, amp drive live, EQ live, Stage reverb, delay feedback FX.
