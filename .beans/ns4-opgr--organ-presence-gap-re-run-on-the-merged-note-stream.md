---
# ns4-opgr
title: 'Organ presence-gap re-run on the merged note stream (B3Wheels flip evidence, D22)'
status: completed
type: task
created_at: 2026-11-22T00:00:00Z
updated_at: 2026-11-22T00:00:00Z
---

## Context
D220 (#596) showed the B3 "too dark" gap was the replayed note set and added `emu_vs_hw.py --merge-gap`. The follow-up named in its note: re-run organ-presence-gap with the merged stream, which may reopen the `B3Wheels` default flip (D22). Stacked on #596 until it merges.

## Higher Goal
1:1 audio parity with the hardware; decide the B3 default on evidence from the right note set.

## Acceptance Criteria
- [x] Re-run B3 Chords n Solo and B3 Old Forest with confirmed, raw and merged streams; per-band deltas before and after
- [x] Say whether the evidence supports flipping `B3Wheels` (no) with an owner-ready proposal and criterion
- [x] Research note, D225, audio-remaining row 9, ROADMAP, data files
- [x] No default flipped, no engine change; AGENTS.md checks pass

## Out of Scope
Flipping `B3Wheels` (owner), Vox re-run, firmware Leslie install (D190), any Rust change, tool changes (the tool and its test come from #596).

## Summary of Changes
- `docs/research/organ-presence-rerun.md`, data `emu_vs_hw_organ_presence_rerun{,_results}.json`, D225, audio-remaining row 9, ROADMAP.
- Result: merged stream B3 Chords Legacy 4.04 against `B3Wheels` 7.22 (presence -3.0 vs -10.9, brilliance +1.3 vs -9.0 dB); Old Forest merged favours `B3Wheels` 4.38 against 5.52; `B3Wheels` stays opt-in.
