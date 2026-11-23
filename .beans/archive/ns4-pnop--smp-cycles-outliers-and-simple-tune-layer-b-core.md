---
# ns4-pnop
title: 'SMP chain tap: check the smp_cycles outliers, bind and rank Simple Tune (layer B)'
status: completed
type: task
created_at: 2026-11-22T12:00:00Z
updated_at: 2026-11-22T12:00:00Z
parent: ns4-trkb
---

## Context
D216 ran the SMP chain-tap census over 155 piano programs. `smp_cycles` had nine outliers (x2 in 4 programs, x0.5 in 2, 0.0003 in 3) that were not checked, and Simple Tune (piano layer B only) had no tap.

## Higher Goal
1:1 audio parity with the firmware: a trustworthy per-program chain tap (pitch and level) for the piano programs.

## Acceptance Criteria
- [x] Per-outlier verdict table: 4 real layer octaves, 5 detector faults ([piano-smp-pitch-outliers](../../docs/research/piano-smp-pitch-outliers.md) s2)
- [x] Tool fixes with tests: `tap_fundamental` (autocorrelation octave), `smp_pitch_audit.py`, layer B core in `arm_piano_bind` (`tools/tests`)
- [x] CSV update (5 cells, no audio committed): audit 148 `ok`, 4 `octave_shift`, 0 `mismatch`
- [x] Simple Tune traced: layer B plays on piano core 2 (3 of 3 descriptors); its bus `0x9405A` is not in the tapped ring
- [x] Research note, D230, audio-remaining row 24, OQ41
- [ ] Rank Simple Tune: needs a tap of the bus words in `sharc-live --smp` (follow-up)

## Out of Scope
Default flips, a bus tap in `sharc-live`, the dry native piano render, a full census rerun.

## Summary of Changes
`tap_fundamental` reads the octave by autocorrelation and refines with the harmonic sum; `tools/smp_pitch_audit.py` checks `smp_cycles` against the layer octave; `arm_piano_bind.bind_piano(layer=)` binds layer B to core 2; five CSV cells corrected; D230 and [piano-smp-pitch-outliers](../../docs/research/piano-smp-pitch-outliers.md) record the evidence.
