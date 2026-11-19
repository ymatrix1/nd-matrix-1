---
# ns4-6zv6
title: Close SAMPLE oracle gaps (level, layers, no-voice, harness quirks)
status: completed
type: task
priority: normal
created_at: 2026-11-17T20:24:10Z
updated_at: 2026-11-17T20:26:20Z
parent: ns4-trkb
---

## Context

PR #506 (D150) measured oracle SAMPLE audio 20 to 23 dB under native, lost layers in 74 programs, never voiced 18 programs and used harness tricks.

## Higher Goal

Native engine audio 1:1 with hardware using only the SHARC oracle, the ARM harness and the native engine.

## Acceptance Criteria

- [x] Level gap decomposed stage by stage for 3 programs (harness omission vs native error)
- [x] Harness quirks (per-loop poke, prepare first, refill-ahead) removed or explained
- [x] Layers and no-voice programs traced; whole-member capture of 119 programs
- [x] Finger Bass octave explained
- [x] docs/research/sample-gaps.md, ROADMAP, audio-remaining, OPEN-QUESTIONS, DECISIONS D154

## Out of Scope

Native engine changes, default flips, the SY receiver stage, the third layer voice.

## Summary of Changes

Level gap: 24.0 to 24.3 dB at the voice stage vs 24.08 dB ring scale (harness omission). Whole-member capture: key 60 voice for 110 of 119 programs (50 before), 86 with every scene I layer. Per-loop poke replaced by pcm credit=on; ahead= option; Finger Bass reading fixed. Open: third layer voice, SY receiver, 3 programs. D154, docs/research/sample-gaps.md.
