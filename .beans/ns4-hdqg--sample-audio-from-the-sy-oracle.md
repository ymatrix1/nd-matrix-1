---
# ns4-hdqg
title: SAMPLE audio from the SY oracle
status: completed
type: task
created_at: 2026-11-17T17:10:11Z
updated_at: 2026-11-17T18:00:00Z
---

## Context

PR #499 showed 110 of 119 SAMPLE programs voice, but nothing measured the audio. D112 left the real packed stream desynced.

## Higher Goal

Native engine audio 1:1 with the hardware, using the SHARC oracle as reference.

## Acceptance Criteria

- [x] 10 programs driven through the SY oracle (decoded PCM ring feed) and measured against native
- [x] The no-voice programs diagnosed; blocker counts for all 119
- [x] Tools with tests; research doc, data, D150, ROADMAP, OQ22 and audio-remaining updated

## Out of Scope

Unpacker fix (real stream), writer of pitch entry `e`, multi-voice and multi-layer, hardware clip comparison (none exists), any default flip.

## Summary of Changes

`smp_stream`, `sharc-ref pcm`, `tools/sample_stream_*.py`, `docs/research/sample-stream-audio.md` (decision D150, PR #506).
