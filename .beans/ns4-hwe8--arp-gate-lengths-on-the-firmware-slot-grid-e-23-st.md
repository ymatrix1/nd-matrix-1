---
# ns4-hwe8
title: Arp gate lengths on the firmware slot grid (E = 2/3 step, plain arp staccato)
status: in-progress
type: task
created_at: 2026-11-15T01:17:10Z
updated_at: 2026-11-18T04:00:00Z
parent: ns4-xc5n
---

## Context

The fifth research pass on the arp (docs/research/synth-arp-patterns.md section 9, OQ32) showed that the firmware plays the note-off of an E step at the second grid point of its step; the pattern objects boot with the grid 0xC02BC1A0 whose second point is at 2/3 of a step, so E lasts 2/3 of a step and Pattern off (all-zero words, every step E) is staccato. The engine keeps E = 4/8 of a step and holds the plain arpeggio note to the next step.

## Higher Goal

Arp / Gate note lengths equal the hardware (1:1 parity).

## Acceptance Criteria

- [x] Decide the meaning of the pattern object flag [obj+0xC]: traced in the ARM harness (D168): written only by the constructor, 0 stores in 41 runs, 2/3 on 53 ARM-played layers of 30 arpeggiator programs. No hardware clip.
- [x] Model E as 2/3 of a step and the plain arpeggio as E steps (gate lengths are stored as eighths: GUI dial 0..8, host parameters, pattern words), with goldens from tools/ns4_arp_player.py events. Opt-in since D166 (`synth/arp_gate.rs`, `--arp-fw-gate`, default off).
- [x] Update synth-engine.md, the gate tags and OQ32.
- [x] Flip the gate default (D168; `--no-arp-gate` opts out).
- [ ] Hardware clip of an arp note against its step (and Hold / scene recall, native Gate-mode retrigger). Do not archive before.

## Out of Scope

Step length and rate table (done in ns4-rwhu); Master Clock sync.

## Summary of Changes

- `synth/arp_gate.rs`: `firmware_gate_fraction` (E = 2/3, q / Q = full step, in-between eighths interpolated, ASSUMED), plain arpeggio releases at 2/3 of a step, `pattern_release` uses the same fraction.
- `FirmwareOptions::arp_firmware_gate` (default off, `--arp-fw-gate` on); D166, OQ32, synth-engine spec and ROADMAP synced. The gate is VERIFIED only in the replayed boot state (30 captures, one state), so it stays opt-in; this bean is reopened, not done.
- 2026-11-19 (D168): flag trace and 32-program census (`tools/arm_arp_flag_trace.py`, `tools/arp_flag_census.py`, `arp-flag-census*.json`); gate default on, `--no-arp-gate` opts out. Bean stays open for the hardware clip.
