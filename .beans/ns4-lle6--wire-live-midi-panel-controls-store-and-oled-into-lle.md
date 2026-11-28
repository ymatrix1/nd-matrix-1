---
# ns4-lle6
title: 'Wire live MIDI, panel controls, .ns4b store and OLED into LLE backend'
status: in-progress
type: task
priority: normal
created_at: 2026-11-23T21:40:00Z
updated_at: 2026-11-23T21:40:00Z
parent: ns4-lle1
blocked_by:
    - ns4-lle5
---

## Context
To be playable as an instrument, the unified LLE engine (`ns4-lle5`) needs live user input (MIDI notes, CCs, sustain/control pedals, pitch stick, mod wheel, panel knob/button events), program switching from the user's `.ns4b` backup (`ns4-pyqg`), and live OLED/LED telemetry (`ns4-2gdc`).

## Higher Goal
A musician can play the LLE engine live from a MIDI keyboard and the ND Matrix 1 panel UI, switch programs from their `.ns4b` backup, and see the real firmware OLED screens and LEDs respond in real time.

## Acceptance Criteria
- [ ] (neutral half and plugin bridge done: `lle_io` rings, events, `LleInput`, `nord-plugin` `lle_bridge` forwards host MIDI with sample offsets; ARM0 queue injection blocked on `ns4-lle5`) Inject live MIDI (note on/off, velocity, channel aftertouch, CC, pitch bend, program change) into ARM0 input queues with lock-free ring buffers
- [ ] (neutral half and plugin side done: `PanelEvent`, `panel_edit`, bridge turns moved host parameters into panel events with placeholder ids; real firmware ids and ARM0 wiring blocked on `ns4-lle5`) Map ND Matrix 1 panel knob/button/drawbar edits to ARM0 panel input events
- [ ] Populate the ARM0 program store from the user's `.ns4b` backup (`ns4-pyqg`)
- [ ] (neutral half and plugin read side done: `TelemetrySnapshot` seqlock channel, `LleTelemetry`, `LleBridge::poll_telemetry`; editor drawing and real data blocked on `ns4-lle5`) Stream real firmware OLED framebuffers (`ns4-2gdc`) and LED states back to the GUI via lock-free telemetry
- [x] Unit tests for MIDI/panel event translation and telemetry snapshots (neutral layer: `crates/nord-core/src/lle_io/`, `tests/no_alloc_lle_io.rs`)
- [x] Plugin bridge behind an opt-in switch (off by default; `NS4_ENGINE=lle` or saved `engine = "lle"`), deterministic loopback fake backend, tests for order and sample offsets, overflow counting, zero allocation in `process()`, and byte-identical audio with the bridge off (`crates/nord-plugin/src/lle_bridge/`, `crates/nord-plugin/tests/lle_bridge.rs`)

## Summary of Changes (so far)
Added `crates/nord-core/src/lle_io/` (SPSC ring, seqlock, events, translation, telemetry, `LleInput` / `LleTelemetry`), `tests/no_alloc_lle_io.rs` and `docs/specs/lle-io.md`. Program store (`ns4-pyqg`) and all ARM0 wiring remain, blocked on `ns4-lle5`.

Plugin bridge: `crates/nord-plugin/src/lle_bridge/` (`LleBridge`, `Stamped` ring items with the host sample offset, `FakeBackend`), `LleInput::handle_event_at`, hooks in `NdMatrix1Plugin::{initialize, process}`, `tests/lle_bridge.rs`. Still open: a real engine to attach (`ns4-lle5`), real firmware panel ids, drawing OLED / LEDs and the status line in the editor, Windows and macOS host runs.

## Out of Scope
Default backend switch (`ns4-lle7`).
