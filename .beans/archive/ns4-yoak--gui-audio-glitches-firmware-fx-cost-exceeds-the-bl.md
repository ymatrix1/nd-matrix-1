---
# ns4-yoak
title: 'GUI audio glitches: firmware FX cost exceeds the block budget (Horizon Whl)'
status: completed
type: task
priority: normal
created_at: 2026-11-18T00:28:34Z
updated_at: 2026-11-18T00:41:04Z
parent: ns4-trkb
---


## Context
The GUI with A:16 Horizon Whl and Engine [tone=Firmware filter=Firmware fx] showed late 9569/189418, worst 75.81/10.00 ms, load 34%, peak -2.4 dB.

## Higher Goal
Glitch-free GUI audio with the firmware paths on.

## Acceptance Criteria
- [x] Deterministic reproduction without a sound card (bench --gui with a real program, arpeggiator, repeat-min timing)
- [x] Cause located with a profile
- [x] Bit-identical speedup (render_hash unchanged) and an opt-in idle skip, with tests
- [x] Docs, DECISIONS, ROADMAP synced; all AGENTS.md checks pass

## Out of Scope
Default flips, firmware or audio contents in git, ARM/SHARC port rewrites.

## Summary of Changes
- `ns4 bench --gui` loads a real program (`--backup/--program`), forces the arpeggiator (`--arp`), keeps each block's fastest of N runs (`--repeat`) and prints what each FX rack runs.
- Finding: the tail spikes are host noise; the render is over budget because every one of the seven racks runs a firmware Cathedral reverb (about 1 ms per rack and 480 frame block) on Horizon Whl; the master bus hard clamps at 1.0 (follow-up).
- `E40` bit-identical fast path (4 to 5 percent) and opt-in `--fx-idle-skip` (Scene I 12 to 15 ms to 2.7 to 3.8 ms, same hash), GUI switch, D161, research note, specs, TD-14, ROADMAP.
