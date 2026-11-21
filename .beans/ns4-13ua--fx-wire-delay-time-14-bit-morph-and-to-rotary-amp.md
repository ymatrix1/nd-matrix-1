---
# ns4-13ua
title: 'FX: wire delay-time (14-bit) morph and To Rotary amp routing'
status: todo
type: task
created_at: 2026-11-12T01:46:47Z
updated_at: 2026-11-12T01:46:47Z
parent: ns4-trkb
---

## Context
delay_rate morph and Amp mode To Rotary are decoded but not applied.

## Acceptance Criteria
- [x] delay_rate morph in FxMorphSet with unit test (D204, `fx/tests_fw_delay_morph.rs`, ARM rows `docs/research/data/morph-fw-delay.csv`)
- [ ] To Rotary routes amp output into organ rotary or documented as unsupported

## Out of Scope
Firmware table work.

## Progress 2026-11-18
Progress (D163, `docs/research/program-ranking-fx.md`): the delay time law (`delay_time`, with `delay_laws`) is in `FirmwareFx::PARITY` (opt-in); the 264 words match the oracle but the delay role has no hardware clip, so it is not flipped.

## Progress 2026-11-21
D204: `FxMorphSet::delay_rate` is loaded, played (firmware sum, clamp, rounding), copied and exported; it reaches the audio only with the opt-in `delay_time`. To Rotary stays open (D195).
