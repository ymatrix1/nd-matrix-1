# Piano extra voices and the damper-down voice list

- **Checked:** 2026-11-16
- **Sources (local, never committed):** the ARM harness (`tools/arm_piano_load.py` library stub, `tools/arm_keydown.py` rig, OS v1.66 image, settled snapshot), the 38 factory `.npno` files of `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (header tables only), `docs/research/data/program-sweep.json`; earlier work [piano-sound-load](piano-sound-load.md) s9 to s11 (D23), [piano-decay-attack](piano-decay-attack.md), [piano-attack-fw](piano-attack-fw.md), [piano-attack-oracle](piano-attack-oracle.md), [smp-slew](smp-slew.md). Derived numbers only.
- **Question:** (1) Which extra voices does the firmware start on a piano note-on (classes 1, 2, 3, 5, 6; audio-remaining item 21) and does the engine play them? (2) Which voice list does the damper-down state select (item 12 residual, D110)?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Result

| Item | Result | Tag |
|---|---|---|
| Item 21 "the engine does not play the extra voice" | **stale.** `piano/extra_voice.rs` has started the extra voice on classes 1, 2, 3, 5, 6 by default since D23 (voice rule 96 of 96, gains 12 of 12, [piano-sound-load](piano-sound-load.md) s10, s11). The coverage row `piano.extra_voice=not_played` predated that reading; it is now `played` (`tools/program_sweep.py`) | VERIFIED (existing tests, re-read) |
| The "layer" byte of a stroke record (`byte 4 & 3`, the selector's list index) is the **stroke kind** | 0 normal, 1 damper-down, 2 key-up; the `.npno` header doc names the same values (`flags_hi`, [nsmp4-and-npno-cnsp-codec](nsmp4-and-npno-cnsp-codec.md) s3.2). So the "extra voice on layer 1" of D23 plays the **damper-down stroke** of the root, and "layer 2" is the key-up record of D103 / D110 | VERIFIED (record dump against the level calls, s2) |
| Extra voice by class | s3 table: damper-down stroke when the root has one (grands and uprights), else a second normal stroke at velocity position `T + 20` (EP, Wurlitzer, EGrand, Bright Grand); none on classes 4, 7, 14, 15, 16 | VERIFIED (s2 run, 38 sounds) |
| **Damper-down voice list** | with the sustain pedal down (CC64 = 127 on UART2 before the note-on) the firmware runs the **same level calls** as with the pedal up: same stroke records, same Q23 gains, same number of voices, on every one of the 38 factory sounds (s2). The pedal changes only the release: the note-off is held and the pedal-up writes the key-up record | VERIFIED (run) |
| Engine, default | pedal-down notes bypass the firmware list and use the legacy damper-stroke path (D79 / `extra_voice.rs`): not what the firmware does | VERIFIED (against s2) |
| Engine, default since D262 | `PianoLayer::firmware_pedal_voices`: pedal-down notes start the same voices as pedal-up notes (and take the slew when `--piano-slew-fw` is on); `--legacy-piano-pedal-voices` restores the damper-stroke path. Issue #679: the legacy path was 6.0 to 6.5 dB quieter (one voice for two). `render_hash` unchanged | engine behaviour tested |

## 2. Method and reproduction

`tools/arm_piano_pedal.py` loads one factory `.npno` through the real firmware loader (library stub), binds it, then plays note 60 at velocity 90 twice from the settled snapshot: plain, and after `B0 40 7F` (CC64 = 127). It hooks the level method `0xC00A1EB4` (and its three sibling classes) and prints one derived row per sound: class byte, root, records of the root per kind, extra voice kind, the level calls (call-frame word `X`, `rec[0x5E]`, key, Q23 output) with and without the pedal, and message counts of the note-on, note-off and pedal-up phases.

```bash
uv run --with unicorn python tools/arm_piano_pedal.py target/fw_unpacked/bank0_CODE.bin \
    target/fw_unpacked/bank0_SRAM.bin --cache target/keydown.snap \
    --ns4b ".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" \
    --npno "Piano/Grand/Velvet Grand Med 6.1.npno" --note 60 --velocity 90
```

Velvet Grand, note 60, velocity 90: two level calls in both runs, `(X 34, rec 36, Q23 3178688)` (the damper-down stroke, extra voice) then `(X 34, rec 35, Q23 3215622)` (the normal stroke, main voice). The record `rec[0x5E]` 36 is the layer-1 record of the root (record 356, threshold 40), 35 the layer-0 record 347 (threshold 34). The SMP message stream of the note-on has the same length (114 words) with and without the pedal; only the slot the key-up record words (`0.25`, `0x3F7FFFFE`) are written to moves, and at the note-off the pedal-down run writes `1.0` words instead of the `0.979` key-up ramp, deferring the key-up record to the pedal-up (4 words).

Sweep of the 38 factory sounds (one invocation per `.npno` listed by the backup): the level calls with the pedal are identical to those without on 38 of 38 sounds; message counts (note-on / note-off / pedal-up phases) are 67 or 65 / 8 / 0 without the pedal and 67 or 65 / 4 / 4 with it on the single-voice classes, 108 or 114 / 8 / 0 and 108 or 114 / 8 / 4 on the two-voice classes (the pedal-up always writes one 4-word key-up record).

## 3. Table per class

Layers use the class in the 384 factory programs (`program-sweep.json`: 276 piano layers of classes 1, 2, 3, 5, 6 in 250 programs). "Roots with damper" counts roots (sampled keys) of the file that carry a damper-down stroke.

| Class | Sounds | Layers (programs) | Extra voice kind | Roots with damper-down stroke | Trigger | Level source | Reproduce |
|---|---|---|---|---|---|---|---|
| 1 (EGrand) | 1 | 5 (5) | second normal stroke at `T + 20` (layer 0) | 0 of 32 | every note-on, pedal up or down | VERIFIED: Q23 word `level_q23 / 2^23` equals the SMP record float (12 of 12, D23); selection 96 of 96 | `arm_piano_pedal.py` row of each sound (s2) |
| 2 (EP1..EP9) | 9 | 55 (52) | second normal stroke at `T + 20` (layer 0) | 0 of 286 | every note-on, pedal up or down | VERIFIED: Q23 word `level_q23 / 2^23` equals the SMP record float (12 of 12, D23); selection 96 of 96 | `arm_piano_pedal.py` row of each sound (s2) |
| 3 (Wurlitzer) | 2 | 8 (8) | second normal stroke at `T + 20` (layer 0) | 0 of 57 | every note-on, pedal up or down | VERIFIED: Q23 word `level_q23 / 2^23` equals the SMP record float (12 of 12, D23); selection 96 of 96 | `arm_piano_pedal.py` row of each sound (s2) |
| 5 (grands) | 7 | 151 (150) | damper-down stroke (kind 1) where the root has one, else normal `T + 20` | 231 of 302 | every note-on, pedal up or down | VERIFIED: Q23 word `level_q23 / 2^23` equals the SMP record float (12 of 12, D23); selection 96 of 96 | `arm_piano_pedal.py` row of each sound (s2) |
| 6 (uprights) | 7 | 57 (50) | damper-down stroke (kind 1) where the root has one, else normal `T + 20` | 229 of 286 | every note-on, pedal up or down | VERIFIED: Q23 word `level_q23 / 2^23` equals the SMP record float (12 of 12, D23); selection 96 of 96 | `arm_piano_pedal.py` row of each sound (s2) |
| 4, 7, 14, 15, 16 | 11 | not counted (main voice only) | none | 0 | | VERIFIED (class byte rule 96 of 96) | same |

Key-up (kind 2) records exist on 34 to 36 of the 38 sounds' roots (Clavinet D6 and Marimba have none); the key-up sample level is the D103 word (`0.21875 * F15`, F15 ASSUMED between velocities 100 and 127). The level call list with the pedal down equals the pedal-up list on **38 of 38** sounds (`same=True` in every row).

## 4. What is not played, and what stays open

- **Extra voice, default engine:** played (all five classes). Muted only under `--piano-slew-fw`: the SMP ramp of the first record is zero (`F-2 = F3 = 0`, [smp-slew](smp-slew.md)), so the slew voice starts the main voice only. Not a gap.
- **Pedal-down notes, default engine:** legacy path (one damper stroke instead of the firmware pair). The opt-in flag plays the firmware list. The engine's own sustain logic holds and releases the voices; the firmware's key-up record placement under the pedal (`1.0` words at the note-off, one `0.979` record at the pedal-up, against two records at a plain note-off) is **not** modelled: the engine spawns one release sample at the pedal-up either way. ASSUMED equivalent until a single-note pedal clip exists.
- Key-up record gain and `F15` law remain D103 (ASSUMED between velocities 100 and 127).

## 5. Against the hardware clips

Emulator against hardware, transcribed velocities and pedal (`python tools/emu_vs_hw.py --only piano_dry piano_felt_tr pianopad_grand --events-from transkun`, Part A video, ns4 built from this branch). Numbers only; nothing flips (D22). Score is the segment score (lower is better); RMS dB = emulator minus hardware.

| Clip | Variant | RMS dB | Attack hw / emu ms | Release dB hw / emu | Score |
|---|---|---|---|---|---|
| Velvet Grand (`piano_dry`) | default | -2.6 | 64.4 / 54.2 | -4.17 / -3.11 | 1.37 |
| | pedal-voices | -0.8 | 64.4 / 55.8 | -4.17 / -2.51 | 2.74 |
| | chain + slew | -5.4 | 64.4 / 68.9 | -4.17 / -2.90 | 3.74 |
| | pedal-voices + chain + slew | -9.5 | 64.4 / 70.8 | -4.17 / -8.31 | 1.53 |
| Felt Upright (`piano_felt_tr`) | default | -4.2 | 55.7 / 54.8 | -4.08 / -3.74 | 1.37 |
| | pedal-voices | -3.4 | 55.7 / 54.8 | -4.08 / -3.80 | 1.33 |
| | chain + slew | -9.7 | 55.7 / 62.1 | -4.08 / -3.99 | 1.31 |
| | pedal-voices + chain + slew | -13.2 | 55.7 / 48.4 | -4.08 / -4.57 | 2.49 |
| Grand Pad (`pianopad_grand`) | default | -1.0 | 57.1 / 56.8 | -0.76 / -4.22 | 1.43 |
| | pedal-voices | +1.1 | 57.1 / 56.2 | -0.76 / -4.27 | 1.51 |
| | piano-chain + slew | -2.5 | 57.1 / 52.4 | -0.76 / -4.72 | 2.61 |
| | pedal-voices + chain + slew | -6.1 | 57.1 / 51.0 | -0.76 / -4.81 | 1.73 |

Reading: `pedal-voices` alone raises the level by +0.8 to +2.1 dB (Velvet -2.6 to -0.8, Felt -4.2 to -3.4, Grand Pad -1.0 to +1.1; all inside the D22 3 dB RMS target before and after). The score worsens on Velvet (1.37 to 2.74: presence +1.9 to +4.1 dB, brilliance +3.9 to +7.7 dB, centroid +3 to +42 Hz), is neutral on Felt (1.37 to 1.33) and slightly worse on Grand Pad (1.43 to 1.51). On the opt-in `chain + slew` paths it improves the score on Velvet (3.74 to 1.53) and Grand Pad (2.61 to 1.73) and worsens it on Felt (1.31 to 2.49), with the level 5 to 13 dB low in every case. No default flips; the slew paths stay opt-in.

## 6. Engine census on all 38 sounds (D202)

`cargo run --release -p nord-tools --bin piano_voice_census -- "<Factory Restore RevK v1.66.ns4b>"` (data: [piano-voice-census.csv](data/piano-voice-census.csv)). Per `.npno`: 30 notes (21 to 108 step 3) x 6 velocities (1, 32, 64, 90, 110, 127) = 6,834 points with a firmware list. The firmware list is `NpnoLevel::voices` (VERIFIED against the ARM level calls, D23 and s2); the engine side is the stroke index and amplitude of the voices `PianoLayer::note_on` starts (tolerance 1e-6).

| Class | Sounds | Points | Two-voice | Damper-down extra | Pedal up = fw | Pedal down default = fw | Pedal down opt-in = fw |
|---|---|---|---|---|---|---|---|
| 1 | 1 | 180 | 180 | 0 | 180 | 0 | 180 |
| 2 | 9 | 1620 | 1620 | 0 | 1620 | 0 | 1620 |
| 3 | 2 | 360 | 360 | 0 | 360 | 0 | 360 |
| 5 | 7 | 1260 | 1260 | 858 | 1260 | 0 | 1260 |
| 6 | 7 | 1260 | 1260 | 954 | 1260 | 0 | 1260 |
| 4 | 1 | 180 | 0 | 0 | 180 | 142 | 180 |
| 7 | 3 | 540 | 0 | 0 | 540 | 540 | 540 |
| 14 | 4 | 714 | 0 | 0 | 714 | 326 | 714 |
| 15 | 2 | 360 | 0 | 0 | 360 | 231 | 360 |
| 16 | 2 | 360 | 0 | 0 | 360 | 156 | 360 |

Reading: tag VERIFIED (deterministic census, tests in `piano_voice_census.rs` and `piano/pedal_voices.rs`; the firmware side rests on s2). Pedal up and the opt-in pedal-down path reproduce the firmware list at 6,834 of 6,834 points. The default pedal-down path plays one legacy voice: it equals the firmware list on 0 of 4,680 two-voice points and on 1,395 of 2,154 single-voice points (it picks a damper-down stroke where the firmware plays the normal one). The extra voice is the damper-down stroke at 1,812 of 2,520 grand and upright points. No mismatch remains outside the default pedal-down path, which the opt-in flag already replaces; no engine change.

## Implications

- Item 21 closes without an engine change: the extra voice is already played. The remaining piano voice-list difference was the pedal-down bypass, which the firmware does not have; the opt-in flag removes it. Defaults stay (D22) until the clips below justify a flip.
- The "damper-down strokes are an ordinary list with a different level" reading of [smp-slew](smp-slew.md) s4 is refined: the damper-down strokes are the **extra voice of the grands and uprights on every note-on, pedal or not**, selected by the layer-1 list; the pedal does not select them.
- The legacy path's use of kind-1 strokes as the main stroke with the pedal down has no firmware counterpart.

## Follow-ups

- Single-note hardware recording with and without pedal: the level of the extra (damper-down) voice and the pedal key-up behaviour.
- Decode the SMP key-up record placement under the pedal (`1.0` words) if the clips show a release difference.
- Default of the pedal-down voices decided: on (D262, issue #679). `--piano-slew-fw` and the chain stay opt-in; the key-up record placement under the pedal is still open.

## 8. Pedal-down level on real sounds (D262, issue #679)

Checked 2026-11-28 with OS v1.66 (`tools/nord_os_urls.py 1.66`, unpacked with `ns4 inspect --unpack`) and the factory backup RevK v1.66, both under `.reference/`.

Sources: `tools/arm_piano_pedal.py` on `Velvet Grand Med 6.1.npno` (note 60, velocity 90): the firmware makes two level calls pedal up and the same two pedal down (`same=True`, `calls_up == calls_down`). `piano_voice_census` on the 38 factory sounds: the engine voice list pedal down equals the firmware list at 6,834 of 6,834 points with `firmware_pedal_voices` (the new default) and at 0 of 4,680 two-voice points on the legacy path (as in s6).

Level, pedal-down over pedal-up, engine output RMS (stereo, 200 ms after the note-on, pedal noise settled), 4 sounds x notes 48, 60, 72 x velocities 32, 64, 110:

| Sound | New default | Legacy path (`--legacy-piano-pedal-voices`) |
|---|---|---|
| Velvet Grand Med 6.1 | 0.00 dB at 9 of 9 points | -4.1 to -6.3 dB |
| Amber Upright | 0.00 dB at 9 of 9 | -3.7 to -6.2 dB |
| Bright Grand | 0.00 dB at 9 of 9 | -4.4 to -6.8 dB |
| Hybrid EP | 0.00 dB at 9 of 9 | -0.5 to +3.1 dB (louder at low velocity) |

The level measurement is a throwaway program (not in the repo); the two commands above are the reproducible part. The EP result shows the legacy path was not uniformly quieter: it differed from the firmware list in both directions.

Update 2026-11-28: the key-up placement under the pedal was measured with `tools/arm_piano_keyup.py` ([piano-pedal-keyup](piano-pedal-keyup.md), D263). The note-off counts above (8 / 4) are the first stage only: the key-up start and the 44 ms release arrive 52 ticks (2.2 ms) after a plain note-off, outside the 10 tick windows of `arm_piano_pedal.py`.
