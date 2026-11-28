# Piano key-up and release placement under the sustain pedal

- **Checked:** 2026-11-28
- **Sources (local, never committed):** the ARM harness (`tools/arm_piano_load.py` library stub, `tools/arm_keydown.py` rig, OS v1.66 image, settled snapshot), the factory `.npno` `Velvet Grand Med 6.1.npno` of `Nord Stage 4 Factory Restore RevK v1.66.ns4b`; earlier work [piano-extra-voices](piano-extra-voices.md) s2 and s8 (D123, D262), [smp-slew](smp-slew.md) s6 (D68, D103), [piano-release-voice](piano-release-voice.md) (D226). Derived numbers only.
- **Question:** (OQ20 residual) When and with which gain, rate and decay does the firmware place the key-up record (stroke kind 2) and release the voices when the key goes up before the pedal lifts, after it, on a re-strike under the pedal, and at a half pedal? Does the engine rule (key-up sample and release at note-off, or at the pedal lift for pedal-held voices) match?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Result

| Case | Firmware (ARM harness, message level) | Engine | Verdict |
|---|---|---|---|
| plain: key-up, no pedal | at +3 / +4 ticks fast-damp writes to two pool records; at **+52 ticks (2.2 ms)** the release write `F16 = 0x3F7FBB6E` (44 ms) on both note voices and the key-up voice start with `F15 = 0x3F7FFFFE`, `F17 = 0x3F7FF38E` (0.2426 s), ramp 0.25 (velocity 90) | release and key-up sample at the note-off | **matches** within 2.2 ms (VERIFIED, s2) |
| (c) pedal lifted while the key is down, key-up afterwards | the lift writes only crossfade ramps; the key-up afterwards is the plain sequence above, same ticks, same words | note-off after the lift is a plain note-off | **matches** (VERIFIED, s2) |
| (a) key-up before the pedal lift | the note-off writes the **hold** word (`F14 = 1.0, F12 = F11 = 0, F16 = 1.0`) to both note voices and starts no key-up. The lift writes one fast-damp record. Neither the 44 ms release nor a key-up start appeared in 12,000 ticks (0.5 s) | voices held at the note-off, released with one key-up at the lift | hold at the note-off **matches**; the placement at the lift is **not observable** in this harness (s4) |
| (b) lift with the key already up | same as (a) | same | not observable (s4) |
| (d) re-strike under the pedal | the second note-on writes the hold word again to the first pair and starts a new pair; the old pair gets no damp | the engine damped the older instance with its soft release | **differs** (VERIFIED message, D263 opt-in `--firmware-piano-pedal-restrike`) |
| (e) half pedal | CC64 = 64 only; 65 to 126 act as pedal up, 127 as full pedal (s5). At 64 the note-on picks a different record for the first voice and the note-off writes `F16 = 0.999958` (1,104 ms) and `0.999814` (247 ms) to the two voices; no key-up start in 800 ticks | `damper_half` slows the release for CC64 values from 0.2 x 127 up | not comparable, not changed (s5) |

## 2. Method and reproduction

`tools/arm_piano_keyup.py` loads one factory `.npno` through the real firmware loader (library stub), binds it, plays note 60 through the UART2 model and records, per phase, the SMP voice record writes the ARM makes (`Message.path == 1`, record array `0x92848 + 54 k + f`): release writes (`F14, F12, F11, F16`; `F16 = 1.0` is the hold word, D68), voice starts (`F-18` plus the stroke words, with level `F15`, rate `F17` and ramp from the voice block `0x9251C` / `0x92528` the pointer word `block - 2` names), and ramp writes (`F3`). One timer tick is 41.67 us (24 kHz, D130, `tools/arm_steady.py`); the older `arm_piano_pedal.py` windows of 10 ticks (0.4 ms) show only the first stage of a note-off, which is why its note-off counts (8 / 4) never contained the key-up start.

```bash
uv run --with unicorn python tools/arm_piano_keyup.py target/fw_unpacked/bank0_CODE.bin \
    target/fw_unpacked/bank0_SRAM.bin --cache target/keydown.snap \
    --ns4b ".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" \
    --npno "Piano/Grand/Velvet Grand Med 6.1.npno" --scenario plain --scenario a --scenario c --scenario d
# the 0.5 s window of case (a): add --scenario a --tail 12000
```

Unit tests: `tools/tests/test_arm_piano_keyup.py` (synthetic messages, no firmware).

## 3. Measured sequences (Velvet Grand Med 6.1, note 60, velocity 90, pedal CC64 = 127 unless noted)

Ticks are absolute within the scenario, us = tick x 41.67; `F16` tau is the per-update decay time constant at 21,701 updates per second (VERIFIED rate, smp-slew s6).

**plain** (`--scenario plain`)

| Tick | Event | Record | Words |
|---|---|---|---|
| 3 | note-on: two voice starts | 80, 81 | levels 0.3789 and 0.3833 (the two level calls), rate `d` 270 and 226 (hold decay `1 - d / 2^24`), ramp 0 and 0.25 |
| 203 / 204 | key-up, stage 1: release write | 93, 106 | `F16 = 0.979331` (tau 2.2 ms), pool records, not the note voices |
| 252 | key-up, stage 2: release write | 80, 81 | `F16 = 0.998954` (tau 44.0 ms) |
| 252 | key-up voice start | 97 | `F15 = 1.0` (`0x3F7FFFFE`), `F17` `d = 3186` (tau 0.2426 s), ramp 0.25 |

**a** (key-up under the pedal, then lift): note-on at the pedal-down starts the pair with the ramp on the **first** (damper-down stroke) voice, 0.25, and 0 on the second (plain: the reverse); note-off at tick 215: `F16 = 1.0` on records 80 and 81 (hold); pedal lift at tick 615: `F16 = 0.979331` on record 93 only. Nothing else in the tail (83 ms with the committed default, 0.5 s with `--tail 12000`).

**a2** (pedal pressed after the strike): the press writes ramps `F3` 0.1417 (record 81) and 0.1598 (record 80) (a crossfade of the two voices); the rest equals (a).

**c** (press, lift, then key-up): lift writes ramps back to 0.25 / 0; the key-up is the plain sequence.

**d** (re-strike): after the first note-off (hold on 80, 81), the second note-on at tick 415 writes the hold word to records 80 and 81 again, and starts records 93 and 94 (levels 0.2415, velocity 70, ramp 0.25 on the first). The second note-off holds 80, 81, 93, 94. The lift writes one fast-damp record (106). No record gets `F16 < 1.0` between the re-strike and the lift.

**half** (CC64 = 64): first voice level 0.3746 (the level call picks record 37 instead of 36) with ramp 0.0238; note-off writes `F16 = 0.999958` (tau 1,104 ms) to record 80 and `0.999814` (247 ms) to record 81; pedal lift: one fast-damp record.

VERIFIED (messages, reproducible command): every row. The tag does not extend to what the SMP does with the words beyond D68 (hold when `F16 = 1.0`).

## 4. Limits of the harness (why (a) and (b) stay open)

1. After a completed key-up the firmware accepts **no further note-on** in this harness: `--scenario again` (note-on, note-off, 2,000 ticks, second note-on at velocity 70) prints no `start` event for the second note in 1,500 ticks. The stage 2 handshake with the SMP is stubbed (`install_dsp_ready`); a record allocation reply is probably missing. Re-strikes can therefore be run only while the first note is pedal-held (case (d)).
2. After the pedal lift of (a) the same holds (`--scenario again_pedal`: no `start` for the second note in 1,500 ticks after the lift). The lift runs stage 1 (one fast-damp record instead of two) but stage 2 (the 44 ms release of the note voices and the key-up start, which the plain note-off queues from a timer-driven job) never ran (`--scenario a --tail 12000`: only the record 93 write at the lift, 0.5 s long). Either the firmware needs the SMP reply here too, or the lift of a pedal-held note queues it by a path the harness does not feed. This note does **not** claim the firmware releases the held voices without a key-up: a piano never sustains forever, so the missing stage is a harness gap until a run shows otherwise.
3. CC64 values 65 to 126 act as pedal up in the harness (s5, `--scenario half --cc N`). The real hardware pedal reaches the same logic through the pedal jack, not MIDI; the MIDI path may be quantised to the values 0, 64 and 127.

## 5. Half pedal and the CC64 map

Values 40, 63, 65, 90, 100 and 126 (`--scenario half --cc N --tail 100`): the plain (pedal up) sequence (note-off stage 2 at +52 ticks with `F16 = 0.998954` and the key-up start on record 97). Not run through the tool: 0 (the plain scenario) and 80 / 110 (exploratory runs of this session, same result). 64: half sequence above. 127: hold. Engine: `HALF_PEDAL_MIN = 0.2` (`engine/pedals.rs`) turns `damper_half` on for CC64 from 26 to 63 and the full pedal at 64 and above (`SUSTAIN_DOWN`). The firmware map (64 half, 127 full, the rest up) is what a pedal jack with three positions gives, not a continuous MIDI law; it is not changed here (no firmware map for the pedal jack in the repository).

## Implications

- Key-up placement without the pedal (plain, (c)): the firmware starts the key-up record 2.2 ms after the note-off, with `F15 = 0.9999` at velocity 90 and the 0.2426 s decay (D226 words). The engine starts it at the note-off: a 2.2 ms offset (106 samples at 48 kHz) against a 44 ms firmware release and an 80 ms engine release; no change (EMPIRICAL judgement, no clip resolves 2 ms).
- Note-off under the pedal (a): the hold word is what the engine does (sustained, no release, no key-up). Match.
- Re-strike under the pedal (d): the firmware keeps the older pair ringing. The engine's damping of the old instance is not firmware behaviour. D263 adds the opt-in `firmware_pedal_restrike` (flag `--firmware-piano-pedal-restrike`). The key-up count after a re-strike is the engine's rule (one per note, ASSUMED).
- The pedal-lift key-up and the half-pedal law remain open because of the harness limits above. OQ20 narrows to those.

## Follow-ups

- Make the harness accept a second note-on after a key-up (answer the SMP allocation reply), then repeat (a), (b) and a plain re-strike; the pedal-lift stage 2 is then observable.
- Decode `0xC00AD4DC` (the stage 1 writer called from the note-off and from the pedal lift) and the timer job `0xC003EDE8` (stage 2) to learn why the lift does not queue it in the harness.
- Single-note hardware recording of a note-off under the pedal, to check the 44 ms release and the key-up level against `F15`.
- Defaults stay (D22): the re-strike hold flips only with a hardware clip and a `render_hash` re-baseline.
