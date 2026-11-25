---
# ns4-lle4
title: 'Live SPORT/audio DMA bus (ORG/PNO/SY/SMP to FX) and SDRAM window'
status: in-progress
type: task
priority: normal
created_at: 2026-11-23T21:40:00Z
updated_at: 2026-11-24T21:00:00Z
parent: ns4-lle1
---

## Context
Currently each DSP chip's audio is captured individually (`--tap` / `--wav` hand-over in `sharc-sys`, `sharc-live`, `sharc-fxlive`). `SPORT` / DAI audio DMA are unmodelled (`docs/specs/sharc-core.md` §6–§8), `FX Core2` falls off its poll loop after 1 pass (`docs/research/pno-jit-arm-profile.md`), and external SDRAM (`0x20xxxxxx`) delay/reverb mapping is still open.

## Gate 1 outcome (2026-11-24)
`ns4-lle3` measured NO-GO for real-time pure-LLE SHARC (slowest core x7.0 to x8.0 over budget; `docs/research/sharc-lle-realtime.md`). This task therefore targets **correct, slower-than-real-time multi-chip LLE** (offline render, null tests against HLE, oracle for Exact mode) and is the base of a **hybrid** (LLE only for cores that fit, Rust HLE for the rest). It must not assume real-time execution.

## Higher Goal
Audio flows live from the tone-generating SHARC chips (`ORG`, `PNO`, `SY1SY2`, `SMP`) through `FX Core1/Core2` (including delay and reverb in external SDRAM) to the final stereo DAC output buffer without WAV hand-overs.

## Acceptance Criteria
- [x] Map and model the external SDRAM window (`0x20xxxxxx`) required by delay and reverb lines (`ns4-8aoy`)
- [x] Fix `FX Core2` poll-loop continuation so both FX cores run continuously across arbitrary pass counts
- [x] Model the inter-chip `SPORT` / audio DMA bus transferring live stereo/multi-bus frames from `ORG`, `PNO`, `SY1SY2`, and `SMP` into `FX Core1/Core2` and out to a host stereo buffer
- [ ] Close remaining blocking peripheral gaps from `ns4-36hw` (`0x28xxxxxx` core aliases, `SIMD + LW` if hit in live audio)
- [x] Unit tests verifying synthetic multi-chip DMA/SDRAM transfer end-to-end

## Out of Scope
ARM0 host integration (`ns4-lle5`); GUI/plugin integration (`ns4-lle7`).

## Summary of Changes
- **FX Core2 poll loop:** the stall was the missing Core1 hand-over counter, not a Core2 defect. `FxDrive` (`runner/fx_run.rs`) presents the counter and the ping selector before every pass; it is a field of `PassJob` (`pass_sched`), used by `fx_live_run`, the scheduler, `lle_sharc_bench` and the bus. Presets `fx1`, `fx2` (`live_presets.rs`). Real images: both FX cores 316 passes idle, 3 370 / 3 220 steps per pass; before: `sharc-sys` FX Core2 pass 1 hit the 200 000-step cap. `lle_sharc_bench` also reads flat ARM load scripts now (`msg::parse_msg_text`).
- **Delay window:** `0x20xxxxxx` loader addresses are the data window `0x0800_0000..0x0804_0000` (`loader = 0x2000_0000 + 4 * (w - 0x0800_0000)`, VERIFIED on both FX zero blocks, 409 600 B = 5 lines of 0x5000 words); dense store `core/sdram.rs` with counters; reverb type 0 run unchanged (null -129.2354 dB vs `FwReverb`, 102 000 reads / 112 016 writes). [sharc-sdram-window](../docs/research/sharc-sdram-window.md).
- **Audio bus:** `runner/audio_bus.rs` (FIFOs, routes with gains, host buffer, one tick per hop), `audio_bus_node.rs` (`CoreNode`, `OutTap`: ORG/PNO lane, FX block/pointer), `audio_bus_sy.rs` (`SyNode` for SY1SY2/SMP), `lle_system.rs` + binary `lle_sharc_bus`. FX Core1 behind the bus is byte identical to `sharc-fxlive` on a real image; ORG Core2 -> FX1 -> FX2 -> host ran 400 ticks idle (silent: no key/effect messages captured). Topology is ASSUMED caller data. [sharc-audio-bus](../docs/research/sharc-audio-bus.md), D247, spec s10.
- **Not done:** `ns4-36hw` gaps (not hit by any run, left unticked); real PNO/SY/SMP image runs through the bus and real tone audio through installed FX chains (need ARM key/effect messages); frame exchange inside the threaded scheduler; real time (Gate 1 NO-GO).
