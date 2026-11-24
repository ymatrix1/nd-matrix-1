---
# ns4-jc3j
title: Synth oscillator level and voice gain per wave, oracle vs native (D140)
status: completed
type: task
created_at: 2026-11-17T14:13:57Z
updated_at: 2026-11-17T14:13:57Z
---

**Context:** D132 left the oscillator level gap open (oracle vs native on 5 programs, unison and layer confound).

**Higher Goal:** audio 1:1 with hardware.

**Acceptance Criteria**
- [x] Oscillator level per wave in dB, voice count matched (unison 0 baseline, unison 1 to 3 ratio)
- [x] Per finding: native constant wrong or oracle input incomplete
- [x] Opt-in fix with tests and clip numbers (--synth-p oracle)

**Out of Scope:** default flips, filter gain, envelope words, WAVE.

## Summary of Changes
See docs/research/synth-osc-gain-oracle.md and D140.
