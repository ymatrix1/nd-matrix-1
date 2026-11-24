---
# ns4-lle4
title: 'Live SPORT/audio DMA bus (ORG/PNO/SY/SMP to FX) and SDRAM window'
status: todo
type: task
priority: normal
created_at: 2026-11-23T21:40:00Z
updated_at: 2026-11-23T21:40:00Z
parent: ns4-lle1
---

## Context
Currently each DSP chip's audio is captured individually (`--tap` / `--wav` hand-over in `sharc-sys`, `sharc-live`, `sharc-fxlive`). `SPORT` / DAI audio DMA are unmodelled (`docs/specs/sharc-core.md` §6–§8), `FX Core2` falls off its poll loop after 1 pass (`docs/research/pno-jit-arm-profile.md`), and external SDRAM (`0x20xxxxxx`) delay/reverb mapping is still open.

## Gate 1 outcome (2026-11-24)
`ns4-lle3` measured NO-GO for real-time pure-LLE SHARC (slowest core x7.0 to x8.0 over budget; `docs/research/sharc-lle-realtime.md`). This task therefore targets **correct, slower-than-real-time multi-chip LLE** (offline render, null tests against HLE, oracle for Exact mode) and is the base of a **hybrid** (LLE only for cores that fit, Rust HLE for the rest). It must not assume real-time execution.

## Higher Goal
Audio flows live from the tone-generating SHARC chips (`ORG`, `PNO`, `SY1SY2`, `SMP`) through `FX Core1/Core2` (including delay and reverb in external SDRAM) to the final stereo DAC output buffer without WAV hand-overs.

## Acceptance Criteria
- [ ] Map and model the external SDRAM window (`0x20xxxxxx`) required by delay and reverb lines (`ns4-8aoy`)
- [ ] Fix `FX Core2` poll-loop continuation so both FX cores run continuously across arbitrary pass counts
- [ ] Model the inter-chip `SPORT` / audio DMA bus transferring live stereo/multi-bus frames from `ORG`, `PNO`, `SY1SY2`, and `SMP` into `FX Core1/Core2` and out to a host stereo buffer
- [ ] Close remaining blocking peripheral gaps from `ns4-36hw` (`0x28xxxxxx` core aliases, `SIMD + LW` if hit in live audio)
- [ ] Unit tests verifying synthetic multi-chip DMA/SDRAM transfer end-to-end

## Out of Scope
ARM0 host integration (`ns4-lle5`); GUI/plugin integration (`ns4-lle7`).
