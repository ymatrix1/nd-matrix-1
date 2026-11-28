---
# ns4-zf7l
title: 'Piano held-note loop (issue #640): loop region, decay and review follow-ups'
status: in-progress
type: task
created_at: 2026-11-27T23:10:42Z
updated_at: 2026-11-27T23:10:42Z
parent: ns4-trkb
---

## Context
Issue #640: a held `.npno` note ended abruptly at the stroke end. D243 faded the tail (owner choice over a loop); PR #675 (D250, renumbered D252) found that the firmware SMP loops the stroke's loop region and decays it by `F+6`, and implemented it. The #675 review left follow-ups.

## Higher Goal
Held piano notes behave like the hardware (loop, decay, no cut) on every path (default, `--piano-slew-fw`, SFZ), with honest provenance tags.

## Acceptance Criteria
- [x] `.npno` strokes loop and decay in the loop region, freed below `1e-4` (PR #675, `piano/sample_loop.rs`)
- [x] SFZ regions loop only for `loop_mode=loop_continuous|loop_sustain`, `loop_end` inclusive, with tests from SFZ text to the voice (`piano/sfz.rs`, `piano/loop_tests.rs`)
- [x] The vacuous `late > 0.0` assertion is replaced by a freed-voice assertion; no decay before the loop start and the in-loop slope are pinned (`piano/loop_tests.rs`)
- [x] `--piano-slew-fw` looped voices take `F+6` in the loop and are stolen by their slew gain (`SlewVoice::set_rate`, `steal_level`)
- [x] D252 and docs reworded ("loop region", `VERIFIED` only for strokes 513 and 502, survey `EMPIRICAL` with method); the SFZ and slew rules are in `specs/piano-engine.md` s9a
- [x] SFZ parser and loop tests moved out of `piano.rs` (behaviour-preserving commits); remaining size in TD-09
- [ ] Owner confirms the reversal of D243 (D252 status note)
- [ ] Hardware recording of a 10 s held note: level at loop entry, `F+7` hold decay, loop seam (OQ20)

## Out of Scope
Predictor history after the wrap, `one_shot` ignoring the key-up, file-embedded SFZ loop points, splitting the rest of `piano.rs` and `cbin/piano_sample.rs` (TD-09).

## Summary of Changes
2026-11-27: follow-ups of the #675 review in one PR (SFZ `loop_mode`, test fixes, slew rate switch and steal priority, docs and tags, D250 renumbered to D252 because D250 was taken twice on `main`).
