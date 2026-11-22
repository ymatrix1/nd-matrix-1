---
# ns4-rmdf
title: 'FX rate match default flip (D180)'
status: completed
type: task
priority: normal
created_at: 2026-11-21T12:00:00Z
updated_at: 2026-11-21T12:00:00Z
parent: ns4-trkb
---

## Context
D137 built the FX rate match (firmware Amp drive and cabinet at the DSP frame rate 43,402 Hz) and D173 recommended flipping it. The owner approved the flip on 2026-11-21.

## Higher Goal
Audio 1:1 with the hardware from firmware and oracle evidence only; the verified FX path is what a user hears by default.

## Acceptance Criteria
- [x] `FirmwareOptions::default()` / `firmware_all()` / every new `LayerFxChain` rate-matched; `--legacy-fx-rate` opts out (CLI, tools, GUI checkbox)
- [x] Tests for the default, the opt-out, unsupported host rates and `apply_exact`
- [x] Six hardware clips, the 384-program ranking and the program sweep re-run on current `main`; worse programs listed (D180, program-ranking-fx s8)
- [x] Golden `render_hash` re-baselined: only segment `fx` changed
- [x] D180, spec, ROADMAP, audio-remaining row 23, gui-testing
- [x] All AGENTS.md checks

## Out of Scope
Reverb ports, the Amp EQ at the frame rate, latency compensation of the converter, hardware clips for the programs that move.

## Summary of Changes
- Default flip and opt-out (`render_opts.rs`, `render_opts_fx_rate.rs`, `fx/mod.rs`, `fx/fw_rate_match.rs`), tools (`--legacy-fx-rate`, `--skip`), GUI checkbox, golden `fx` segment, research data and notes, D180.
