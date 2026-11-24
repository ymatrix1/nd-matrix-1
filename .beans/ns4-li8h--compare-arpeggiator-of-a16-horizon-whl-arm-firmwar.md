---
# ns4-li8h
title: 'Compare arpeggiator of A:16 Horizon Whl: ARM firmware vs native engine'
status: completed
type: task
created_at: 2026-11-17T23:43:54Z
updated_at: 2026-11-17T23:43:54Z
parent: ns4-trkb
---

## Context
Owner reports factory A:16 Horizon Whl sounds very different from the hardware video. Layers A and C have the arpeggiator on (rate raw 90 / 96, file BPM 266 / 318, global tempo 100). D81 notes the clip comparison never included the arpeggiator or the mod wheel.

## Higher Goal
Audio 1:1 with hardware using only ARM + SHARC firmware execution.

## Acceptance Criteria
- [x] ARM note events (note, velocity, gate on/off in 24 kHz ticks) recorded for a held chord and a single key, layers A and C
- [x] Native arp output compared (order, step ms, gate ms; rate raw, file BPM, global tempo 100, MST CLK, hold, kb sync)
- [x] Video segment with audible arp located; step period from onsets
- [x] docs/research/arp-fw-vs-native.md + data under docs/research/data/
- [x] Fixes only behind opt-in flags; tests; DECISIONS entry; ROADMAP/OQ sync
- [x] All AGENTS.md checks pass; PR opened

## Out of Scope
Default flips (owner decides). Committing firmware, samples or audio.

## Summary of Changes
- `tools/arm_arp_events.py` records the ARM arp listener and voice events; `tools/arp_fw_compare.py` compares them with the native `arp_trace` bin; `tools/arp_video_onsets.py` measures the video step (150.0 ms).
- `docs/research/arp-fw-vs-native.md` and `docs/research/data/arp-*.json`: step equal (150 ms), gate 2/3 vs full step, single-note retrigger and UpDown octave walk differ.
- Opt-in `SynthLayer::arp_firmware_walk` (`arp_trace --arp-fw-walk`) with tests. D162, OQ39, ROADMAP 7b.6 synced. No default changes.
- Follow-up 2026-11-18 (D166): the walk and the single-note retrigger are the default (`--legacy-arp` opts out), the 2/3 gate stays opt-in (`--arp-fw-gate`); 30 range / direction captures in `docs/research/data/arp-fw-range.json`.
