---
# ns4-u2ge
title: Reverb stage gain word, routine 0x1211C1 and the 43,402 Hz rate
status: completed
type: task
priority: normal
created_at: 2026-11-17T23:11:26Z
updated_at: 2026-11-17T23:29:25Z
parent: ns4-trkb
---


## Context
D145 / reverb-fw-vs-hw: firmware multi-tap reverb (types 2-11) equals the oracle but sounds 15 dB quieter, 1.2 s tail, darker. Open: stage gain word, routine 0x1211C1, real reverb rate, whether +21/+26/+36 dB is noise floor or gain.

## Higher Goal
Audio 1:1 with hardware from ARM+SHARC firmware execution only.

## Acceptance Criteria
- [x] Wet level and send/return routing located: stage law + send/dry words done; the stage level WRITER is not reached by the ARM harness (open, OQ21c)
- [x] 0x1211C1 decoded; real reverb sample rate settled
- [x] Noise floor vs real gain settled with a deterministic test
- [x] docs/research finding, DECISIONS entry + Index row, OQ/ROADMAP sync
- [x] Any native change opt-in only

## Out of Scope
Default flips (owner OK needed); committing firmware/sample/audio contents.

## Summary of Changes

Stage law verified in the oracle (no gain above unity), kRvbAmt writes only send/dry (module reads only send), 0x1211C1 decoded (2-frame filter-free variant), core rate 21,701 Hz (tail 1.32 s real), excess = signal-relative floor (tests). No native change. D157, docs/research/reverb-stage-gain.md.
