---
# ns4-yp85
title: Apply the program Touch velocity curves (Piano/Organ/Synth) from the firmware
status: completed
type: task
created_at: 2026-11-15T04:15:47Z
updated_at: 2026-11-18T03:00:00Z
---

## Context
The emulator passes the velocity of a note almost unchanged to the engine (MIDI controller velocity as sent; mouse clicks use a fixed 88). The Nord Stage 4 programs store a per-section velocity response (for example `Touch: Medium` on Piano A:13 Felt Upright; Soft / Medium / Hard and the Organ and Synth equivalents). That response is part of the sound and applies to any note source, including an external USB MIDI controller. No model of these Touch curves was found in the engine. The keybed profile curve of the hardware (`[manager+0x134]`, profile types 1 and 2, raw key time 34..416 to velocity byte) only matters for the Nord's own keybed, not for external MIDI, and is out of scope.

## Higher Goal
Closer audio parity: the same program and the same MIDI velocity must give the same level and timbre as the hardware, which includes the Touch setting. Also needed to interpret the Felt Upright and Velvet Grand clip comparisons (uniform velocity 88 is an estimate).

## Acceptance Criteria
- [x] Find in the ARM firmware where the per-program Touch setting (Piano, Organ, Synth sections) is read and how the velocity is mapped (curve tables, edit ids, which core applies it).
- [x] Verify whether the curve applies to incoming MIDI velocity as well as to the keybed (run the real note-on path in the harness for the three Touch values).
- [x] Implement the verified curves in the engine, opt-in first (FirmwareOptions switch and a GUI Firmware bar control); the default flip waits for the decoder realignment (D88, D22): the clips are Medium and no program from a file carries its Touch yet.
- [x] Verify the per-program Touch bits against the firmware recall for every factory program (D165: 384 of 384, `tools/arm_program_touch_bits.py`) and flip the default (`--legacy-piano-touch` opts out).
- [x] Unit tests with oracle/ARM goldens, docs (piano-engine spec, OQ20, ROADMAP, D88, research note; Organ and Synth have no Touch, nothing to document in their specs).

## Out of Scope
The Nord keybed raw-timing to velocity profile curve (hardware keybed only). MIDI port selection (separate task).

## Summary of Changes
- Firmware: Touch is edit id `0x159` of the Piano section only (values 0 heavy / 1 medium / 2 light); handler `0xC011AA24` stores `curveId` 10 / 11 / 13 in the layer object, the voice init reads `T = byte[0xC0532664 + (curveId << 10) + r8]`. It applies to MIDI notes. Organ and Synth have no Touch id. 27 of 27 `T` bytes equal the composed tables in the ARM run; the program file field is at file bit `1991` (layer A) / `2087` (layer B) and equals the edit value (6 of 6 in the program recall).
- Code: `piano/touch_curve.rs` (`TouchCurve`, derived tables, fingerprint test), `piano/touch_layer.rs`, `NpnoLevel::*_touch`, `PianoLayer::firmware_touch`, `FirmwareOptions::piano_touch` (`--firmware-piano-touch`), GUI Firmware bar switch, `tools/arm_touch_curves.py`. Defaults and `render_hash` unchanged.
- Docs: `docs/research/piano-touch-curves.md`, D88, piano-engine spec, OQ20, ROADMAP, gui-testing.
- Decoder follow-up (D99): `cbin/piano_preset.rs` reads variation, `kAcoustic`, pedal noise, Touch, unison, dyn comp and EQ at the firmware positions (`cbin/piano_preset_layout.rs` tests on 12 factory blocks); the default stays off (no Light / Heavy clip). The System-menu MIDI velocity curves are not modelled.
- Default flip (D165, 2026-11-18): `tools/arm_program_touch_bits.py` recalls all 384 factory programs through the firmware; the Touch of both layers equals the decoder in 384 of 384 programs (768 of 768 layer values). `PianoLayer::firmware_touch` / `FirmwareOptions::piano_touch` are on by default, `--legacy-piano-touch` opts out (owner approval 2026-11-18). `render_hash` unchanged; 82 programs with an active Heavy or Light layer move RMS -1.59 .. +1.23 dB (`program_sweep --piano-touch` against `--legacy-piano-touch`). Open: Heavy / Light hardware clip, System-menu MIDI velocity curves.
