---
# ns4-j94m
title: PUMP follows the Master Clock
status: completed
type: task
created_at: 2026-11-17T06:06:23Z
updated_at: 2026-11-17T06:06:23Z
---

Context: firmware PUMP free-runs at 6.1 Hz, hardware 2.11 Hz. Acceptance: counter driven from Master Clock, tests, docs (D134).

## Higher Goal
Audio 1:1 with hardware for the 9 factory programs that use PUMP.

## Acceptance Criteria
- [x] Counter `c[-0x15]` ticked once per Master Clock division behind `FirmwareFx::mod1`
- [x] Period at 126 BPM 2.10 Hz (hardware 2.11 Hz), divisions scale, free run unchanged (tests)
- [x] D134, OQ36 narrowed, ROADMAP, fx-mod-types s7.2, effects-and-routing
- [ ] ARM / DSP counter writer (stays open, OQ36)

## Out of Scope
Default flips, `render_hash`, `synth/fw_filter*.rs`, aot/jit, the hardware writer.

## Summary of Changes
`fx/fw_pump_clock.rs` (tick generator, division increment), `fw_mod1_rest.rs` (`set_clock_period`), `modulation.rs` routing, `tests_fw_pump_sync.rs`, docs.
