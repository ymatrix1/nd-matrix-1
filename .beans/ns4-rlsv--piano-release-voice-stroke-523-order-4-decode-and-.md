---
# ns4-rlsv
title: 'Piano release voice (SMP stroke 523): order 4 decode, key-up record, opt-in engine law'
status: completed
type: task
created_at: 2026-11-22T00:00:00Z
updated_at: 2026-11-22T00:00:00Z
---

## Context
`docs/research/audio-remaining.md` row 24 and OQ41 listed the release voice (SMP stroke 523, order 4 page) as untraced; `tools/smp_voice_replay.py` had no order 4 row. The native key-up sample played `0.30 * level` with a 1.15 s decay (ASSUMED).

## Higher Goal
1:1 audio parity of the piano with the firmware (SMP oracle).

## Acceptance Criteria
- [x] Release voice (voice 17, stroke 523) replayed on the oracle: history ring equals the reference decode (order 4 row `(-1, 4, -6, 4)`)
- [x] Key-up record trajectory (`F0`, `F15`) equals the Rust slew model within 1 LSB, golden test
- [x] Native vs firmware key-up level and decay quantified on 5 programs
- [x] Opt-in flag `--piano-keyup-fw` (`PianoLayer::firmware_key_up`), default unchanged, tests
- [x] Research note, D224, spec, audio-remaining and OQ41 updated

## Out of Scope
Default flip; the SMP tap level of the release voice (tap silent after about 3,800 frames in the harness); `layer_b`; the damper-down pedal key-up placement.

## Summary of Changes
`tools/smp_voice_replay.py` order 4 row + test; `slew_voice.rs` `key_up_decay_tau_s` + oracle golden; `piano.rs` `firmware_key_up`; `render_opts.rs` `--piano-keyup-fw`; `docs/research/piano-release-voice.md`; D224; `specs/piano-engine.md` s6.1.
