---
# ns4-lvl1
title: 'Settle the level link of the firmware envelope law (D236)'
status: completed
type: task
priority: normal
created_at: 2026-11-23T12:00:00Z
updated_at: 2026-11-23T12:00:00Z
---

## Context
D223 kept `--synth-fw-env-law` opt-in because `synth_lead`, `synth_silkpad` and `synth_horizon` get worse; the ASSUMED level link was blamed.

## Higher Goal
Flip defaults only when validated against the firmware and matching; find what stands between the verified word and the clips.

## Acceptance Criteria
- [x] Level link measured on the oracle (20 rows, VERIFIED tag with test)
- [x] Three clips re-scored with the oracle unit and without
- [x] Flip proposal: none (lead and pad worse, `UNIT_2F` ASSUMED), stated in D236
- [x] Research note, D236, spec s4.7, ROADMAP, tests
- [x] All AGENTS.md checks

## Out of Scope
Default flip, refitting the cutoff factors, oracle audio of the pad.

## Summary of Changes
Tool `synth_env_level_link.py` + tests, Rust test and tags in `fw_cutoff_env.rs`, data CSVs, research note, D236. No behaviour change.
